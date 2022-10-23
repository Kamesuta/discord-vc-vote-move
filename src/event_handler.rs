use std::{str::FromStr, sync::Arc};

use crate::app_config::AppConfig;
use anyhow::{anyhow, Context as _, Result};

use dyn_fmt::AsStrFormatExt;
use futures::future::try_join_all;
use log::{error, warn};
use regex::{Match, Regex};
use serenity::{
    json::Value,
    model::{
        application::command::Command,
        application::interaction::Interaction,
        gateway::Ready,
        id::ChannelId,
        prelude::{
            command::CommandOptionType,
            interaction::{
                application_command::{ApplicationCommandInteraction, CommandDataOption},
                InteractionResponseType,
            },
            ChannelType, CommandId, Reaction, UserId,
        },
        user::User,
    },
};

use serenity::async_trait;
use serenity::prelude::*;

#[derive(Clone, Debug)]
/// コマンド
struct Commands {
    /// 部屋を作成して一緒に移動コマンド
    move_command: CommandId,
    /// すでに作成されている部屋に移動コマンド
    move_to_command: CommandId,
}

// コマンドの種類
enum CommandType {
    Move(String),
    MoveTo(ChannelId),
}

impl CommandType {
    /// 文字列に変換
    fn to_string(&self) -> String {
        match self {
            CommandType::Move(channel_name) => format!("新規VC「{}」", channel_name),
            CommandType::MoveTo(channel_id) => format!("{}", channel_id.mention().to_string()),
        }
    }

    /// 文字列から変換
    fn parse(move_to_match: Option<Match>, move_match: Option<Match>) -> Option<Self> {
        move_to_match
            .and_then(|m| {
                ChannelId::from_str(m.as_str())
                    .ok()
                    .map(|channel_id| CommandType::MoveTo(channel_id))
            })
            .or_else(|| move_match.and_then(|m| Some(CommandType::Move(m.as_str().to_string()))))
    }
}

/// イベント受信リスナー
pub struct Handler {
    /// 設定
    app_config: AppConfig,
    /// 登録したコマンドのID
    move_command_id: Arc<Mutex<Option<Commands>>>,
    /// 募集メッセージ
    vote_message: String,
    /// 募集メッセージの正規表現
    vote_message_regex: Regex,
}

impl Handler {
    /// コンストラクタ
    pub fn new(app_config: AppConfig) -> Result<Self> {
        let vote_message = "{}が一緒に移動する人の募集を開始しました。\n{}に移動したい人は{}分以内にリアクション押してください！";
        let vote_message_escape =
            regex::escape(&vote_message.replace("{}", "%s")).replace("%s", "{}");
        let vote_message_with_regex =
            vote_message_escape.format(&[r"<@(\d+)>", r"(?:<#(\d+)>|新規VC「(.+)」)", r"(?:\d+)"]);
        let vote_message_regex = Regex::new(&format!("{}$", vote_message_with_regex))
            .context("募集メッセージの正規表現のコンパイルに失敗")?;
        Ok(Self {
            app_config,
            move_command_id: Arc::new(Mutex::new(None)),
            vote_message: vote_message.to_string(),
            vote_message_regex,
        })
    }

    /// コマンドが呼ばれたときの処理
    async fn register_command(&self, ctx: &Context) -> Result<()> {
        // moveコマンドを登録
        let move_command = Command::create_global_application_command(&ctx, |command| {
            command
                .name("move")
                .description("みんなでVCを移動する投票ボタンを作成します")
                .create_option(|option| {
                    option
                        .name("channel_name")
                        .description("新規作成するチャンネル名")
                        .kind(CommandOptionType::String)
                        .required(true)
                })
        })
        .await
        .context("コマンドの登録に失敗")?;

        // move_toコマンドを登録
        let move_to_command = Command::create_global_application_command(&ctx, |command| {
            command
                .name("move_to")
                .description("みんなでVCを移動する投票ボタンを作成します")
                .create_option(|option| {
                    option
                        .name("channel")
                        .description("移動先のチャンネル")
                        .kind(CommandOptionType::Channel)
                        .channel_types(&[ChannelType::Voice])
                        .required(true)
                })
        })
        .await
        .context("コマンドの登録に失敗")?;

        // 登録したコマンドを保存
        self.move_command_id.lock().await.replace(Commands {
            move_command: move_command.id,
            move_to_command: move_to_command.id,
        });

        Ok(())
    }

    /// コマンドが呼ばれたときの処理
    async fn on_move_command(
        &self,
        ctx: &Context,
        interaction: &ApplicationCommandInteraction,
    ) -> Result<()> {
        // コマンドを取得
        let command_id = self
            .move_command_id
            .lock()
            .await
            .as_ref()
            .context("コマンドが登録されていません")?
            .clone();

        // データを取得
        let data_option: &CommandDataOption = &interaction.data.options[0];
        // 指定されたチャンネルIDを取得
        let channel_str: &str = match data_option.value.as_ref() {
            Some(Value::String(channel)) => channel.as_str(),
            _ => return Err(anyhow!("チャンネルが指定されていません")),
        };

        // コマンドの種類を取得
        let command_type = match interaction.data.id {
            // moveコマンドの場合
            id if id == command_id.move_command => {
                // チャンネル名を取得
                let channel_name = channel_str.to_string();
                // コマンドの種類を取得
                CommandType::Move(channel_name)
            }
            // move_toコマンドの場合
            id if id == command_id.move_to_command => {
                // チャンネルIDを取得
                let channel_id = ChannelId::from_str(channel_str)
                    .map_err(|_why| anyhow!("チャンネルが取得できません"))?;

                // 権限を確認
                let channel = channel_id
                    .to_channel(&ctx)
                    .await
                    .context("チャンネルが取得できません")?
                    .guild()
                    .context("DMチャンネルは取得できません")?;
                if !channel
                    .permissions_for_user(&ctx, interaction.user.id)
                    .context("権限の取得に失敗")?
                    .connect()
                {
                    return Err(anyhow!("指定されたVCに入る権限がありません"));
                }

                // コマンドの種類を取得
                CommandType::MoveTo(channel_id)
            }
            _ => return Err(anyhow!("コマンドが不正です")),
        };

        // 送信者を取得
        let member = interaction
            .member
            .as_ref()
            .ok_or_else(|| anyhow!("送信したユーザーを取得できませんでした"))?;

        // ギルドIDを取得
        let guild_id = interaction
            .guild_id
            .ok_or_else(|| anyhow!("サーバーが見つかりません"))?;
        // ギルドを取得
        let guild = guild_id
            .to_guild_cached(&ctx)
            .ok_or_else(|| anyhow!("サーバーの取得に失敗しました"))?;

        // 送信者がボイスチャンネルにいるか確認
        let voice_channel_id = guild
            .voice_states
            .get(&member.user.id)
            .and_then(|voice_state| voice_state.channel_id)
            .ok_or_else(|| anyhow!("ボイスチャンネルに参加していません"))?;

        // VCのメンバーを取得
        let voice_member_mentions = guild
            .voice_states
            .iter()
            .filter(|(_, state)| state.channel_id == Some(voice_channel_id))
            .map(|(id, _)| id.mention().to_string())
            .collect::<Vec<String>>()
            .join("");

        // メッセージを構築
        let vote_message = self.vote_message.format(&[
            &interaction.user.mention().to_string(),
            &command_type.to_string(),
            &self.app_config.discord.move_timeout_minutes.to_string(),
        ]);

        // メッセージを送信
        let message = interaction
            .channel_id
            .send_message(&ctx, |m| {
                m.content(format!(
                    "{}にいる皆さん({})へ\n\n{}",
                    voice_channel_id.mention(),
                    voice_member_mentions,
                    vote_message,
                ));
                m
            })
            .await
            .map_err(|_why| anyhow!("メッセージの投稿に失敗しました"))?;
        // リアクションを付与
        message
            .react(&ctx, '🤚')
            .await
            .map_err(|_why| anyhow!("リアクションの追加に失敗しました"))?;

        // 一定時間後にメッセージを削除
        let minutes = self.app_config.discord.move_timeout_minutes;
        let ctx_clone = ctx.clone();
        tokio::task::spawn(async move {
            // minutes分後に削除
            tokio::time::sleep(std::time::Duration::from_secs(60 * minutes)).await;

            // メッセージを削除
            match message.delete(ctx_clone).await {
                Ok(_) => {}
                Err(why) => {
                    error!("メッセージの削除に失敗しました: {}", why);
                }
            }
        });

        // 返信をする
        interaction
            .create_interaction_response(&ctx, |response| {
                response
                    .kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|message| {
                        message.ephemeral(true);
                        message.content(format!("一緒に移動する人の募集を開始しました。\nあなたが🤚をつけると、🤚つけた人と一緒に{}へ移動します。", command_type.to_string()));
                        message
                    })
            })
            .await
            .map_err(|_why| anyhow!("リアクションの反応に失敗しました"))?;

        Ok(())
    }

    /// リアクションが押されたときの処理
    async fn on_move_reaction(&self, ctx: &Context, reaction: &Reaction) -> Result<()> {
        // リアクションを追加したメッセージを取得
        let message = reaction
            .channel_id
            .message(&ctx, reaction.message_id)
            .await
            .context("メッセージの取得に失敗")?;

        // リアクションのメッセージがBotのメッセージでなければ無視
        if message.author.id != ctx.cache.current_user_id() {
            return Ok(());
        }

        // メッセージが特定の文字を含んでいなければ無視
        if !message
            .content
            .contains("一緒に移動する人の募集を開始しました")
        {
            return Ok(());
        }

        // リアクションをしたユーザーを取得
        let user_id = reaction.user_id.context("ユーザーIDの取得に失敗")?;

        // メッセージのメンションユーザーを取得
        let caps = self
            .vote_message_regex
            .captures(&message.content)
            .context("メッセージのパースに失敗")?;
        let mention_user = caps
            .get(1)
            .and_then(|m| UserId::from_str(m.as_str()).ok())
            .context("送信者のメンション取得に失敗")?;

        // リアクションを追加した人がメンションされた人でなければ無視
        if mention_user != user_id {
            return Ok(());
        }

        // メッセージのメンションチャンネルを取得
        let mention_channel_id = CommandType::parse(caps.get(2), caps.get(3))
            .context("移動先VCのチャンネル取得に失敗")?;

        // リアクションを追加した人がボイスチャンネルにいるか確認
        let guild_id = reaction.guild_id.context("サーバーの取得に失敗")?;
        let guild = guild_id
            .to_guild_cached(&ctx)
            .context("サーバーの取得に失敗")?;
        let voice_state = guild
            .voice_states
            .get(&user_id)
            .context("ボイスチャンネルに参加していません")?;
        let _voice_channel_id = voice_state
            .channel_id
            .context("ボイスチャンネルのIDの取得に失敗")?;

        // リアクションを追加した人リストを取得
        let reaction_users = reaction
            .users(&ctx, '🤚', None, None::<UserId>)
            .await
            .context("リアクションを追加したユーザーの取得に失敗")?
            .into_iter()
            .filter(|user| user.id != ctx.cache.current_user_id())
            .collect::<Vec<User>>();

        // 移動先チャンネルを取得/作成
        let to_channel_id = match mention_channel_id {
            CommandType::MoveTo(channel_id) => {
                // 権限を確認
                let channel = channel_id
                    .to_channel(&ctx)
                    .await
                    .context("チャンネルが取得できません")?
                    .guild()
                    .context("DMチャンネルは取得できません")?;
                if !channel
                    .permissions_for_user(&ctx, user_id)
                    .context("権限の取得に失敗")?
                    .connect()
                {
                    return Err(anyhow!("指定されたVCに入る権限がありません"));
                }

                channel_id
            }
            CommandType::Move(channel_name) => {
                // メンバーを取得
                let member = guild
                    .member(&ctx, user_id)
                    .await
                    .context("メンバーの取得に失敗")?;

                // まず一人移動
                member
                    .move_to_voice_channel(&ctx, &self.app_config.discord.vc_create_channel)
                    .await
                    .context("移動に失敗")?;

                // すこし待つ
                tokio::time::sleep(std::time::Duration::from_secs(
                    self.app_config.discord.move_wait_seconds,
                ))
                .await;

                // VCの状態が変わっているため、ギルドを再取得
                let guild = guild_id
                    .to_guild_cached(&ctx)
                    .context("サーバーの取得に失敗")?;

                // メンバーが移動した先のチャンネルを取得
                let voice_state = guild
                    .voice_states
                    .get(&user_id)
                    .context("ボイスチャンネルに参加していません")?;
                let voice_channel_id = voice_state
                    .channel_id
                    .context("ボイスチャンネルのIDの取得に失敗")?;

                // 除外対象か確認
                if self
                    .app_config
                    .discord
                    .vc_ignored_channels
                    .contains(&voice_channel_id)
                {
                    return Err(anyhow!("除外対象のチャンネルです"));
                }

                // チャンネルを取得
                let mut channel = voice_channel_id
                    .to_channel(&ctx)
                    .await
                    .context("チャンネルの取得に失敗")?
                    .guild()
                    .context("チャンネルがサーバーのチャンネルではありません")?;

                // 設定したカテゴリの中か確認
                if channel.parent_id != Some(self.app_config.discord.vc_category) {
                    return Err(anyhow!("カテゴリが違います"));
                }

                // VCの名前を変更
                channel
                    .edit(&ctx, |c| c.name(channel_name))
                    .await
                    .context("チャンネルの名前の変更に失敗")?;

                voice_channel_id
            }
        };

        // リアクションをした人全員をボイスチャンネルに移動
        let members = try_join_all(
            reaction_users
                .iter()
                // 通話状態を取得
                .filter_map(|user| guild.voice_states.get(&user.id))
                // メンバーを取得
                .map(|voice_state| guild.member(&ctx, voice_state.user_id)),
        )
        .await?;

        // メンバーを移動
        for member in &members {
            // リアクションを追加した人がボイスチャンネルにいる場合は移動
            let _ = member.move_to_voice_channel(&ctx, to_channel_id).await;
        }

        // 募集のメッセージを削除
        message
            .delete(&ctx)
            .await
            .context("メッセージの削除に失敗")?;
        // 結果を送信
        reaction
            .channel_id
            .send_message(&ctx, |message| {
                message.content(format!(
                    "{}と一緒に{}人のメンバーを{}へ移動しました。",
                    mention_user.mention(),
                    members.len() - 1,
                    to_channel_id.mention(),
                ));
                message.embed(|embed| {
                    embed.title("移動したメンバー");
                    embed.description(
                        members
                            .iter()
                            .map(|member| member.mention().to_string())
                            .collect::<Vec<String>>()
                            .join("\n"),
                    );
                    embed
                });
                message
            })
            .await
            .context("メッセージの送信に失敗")?;

        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    /// 準備完了時に呼ばれる
    async fn ready(&self, ctx: Context, data_about_bot: Ready) {
        // コマンドを登録
        match self.register_command(&ctx).await {
            Ok(_) => {}
            Err(why) => {
                println!("コマンドの登録に失敗しました。: {}", why)
            }
        }

        // ログインしたBotの情報を表示
        warn!("Bot準備完了: {}", data_about_bot.user.tag());
    }

    /// コマンドが実行されたときに呼ばれる
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // 不明なインタラクションは無視
        match interaction {
            Interaction::ApplicationCommand(interaction) => {
                match self.on_move_command(&ctx, &interaction).await {
                    Ok(_) => {}
                    Err(why) => {
                        match interaction
                            .create_interaction_response(&ctx, |response| {
                                response
                                    .kind(InteractionResponseType::ChannelMessageWithSource)
                                    .interaction_response_data(|message| {
                                        message.ephemeral(true);
                                        message.content(why.to_string());
                                        message
                                    })
                            })
                            .await
                        {
                            Ok(_) => {}
                            Err(why) => {
                                error!("エラーメッセージの送信に失敗: {:?}", why);
                            }
                        }
                    }
                }
            }
            _ => return,
        };
    }

    /// リアクションを追加したときに呼ばれる
    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        match self.on_move_reaction(&ctx, &reaction).await {
            Ok(_) => {}
            Err(why) => {
                error!("リアクションの反応に失敗: {:?}", why);
                return;
            }
        }
    }
}
