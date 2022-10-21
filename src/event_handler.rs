use std::{str::FromStr, sync::Arc};

use crate::app_config::AppConfig;
use anyhow::{anyhow, Context as _, Result};

use log::{error, warn};
use regex::Regex;
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
            ChannelType, Reaction, UserId,
        },
        user::User,
    },
};

use serenity::async_trait;

use serenity::prelude::*;

/// イベント受信リスナー
pub struct Handler {
    /// 設定
    app_config: AppConfig,
    /// 登録したコマンドのID
    move_command_id: Arc<Mutex<Option<Command>>>,
}

impl Handler {
    /// コンストラクタ
    pub fn new(app_config: AppConfig) -> Result<Self> {
        Ok(Self {
            app_config,
            move_command_id: Arc::new(Mutex::new(None)),
        })
    }

    /// コマンドが呼ばれたときの処理
    async fn on_move_command(
        &self,
        ctx: &Context,
        interaction: &ApplicationCommandInteraction,
    ) -> Result<()> {
        // データを取得
        let global: &CommandDataOption = &interaction.data.options[0];
        // 指定されたチャンネルIDを取得
        let channel_str: &str = match global.value.as_ref() {
            Some(Value::String(channel)) => channel.as_str(),
            _ => return Err(anyhow!("チャンネルが指定されていません")),
        };
        // チャンネルIDを取得
        let channel_id = ChannelId::from_str(channel_str)
            .map_err(|_why| anyhow!("チャンネルが取得できません"))?;
        // チャンネルを取得
        let channel = channel_id
            .to_channel(&ctx)
            .await
            .map_err(|_why| anyhow!("チャンネルが取得できません"))?;

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
        guild
            .voice_states
            .get(&member.user.id)
            .ok_or_else(|| anyhow!("ボイスチャンネルに参加していません"))?;

        // メッセージを送信
        let message = interaction
            .channel_id
            .send_message(&ctx, |m| {
                m.content(format!(
                    "{}が一緒に移動する人の募集を開始しました。\n{}に移動したい人は{}分以内にリアクション押してください！",
                    interaction.user.mention(),
                    channel.mention(),
                    self.app_config.discord.move_timeout_minutes,
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
                        message.content(format!("一緒に移動する人の募集を開始しました。\nあなたが🤚をつけると、🤚つけた人と一緒に{}へ移動します。", channel.mention()));
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
        let mention_user: &User = message
            .mentions
            .first()
            .context("ユーザーメンションの取得に失敗")?;

        // リアクションを追加した人がメンションされた人でなければ無視
        if mention_user.id != user_id {
            return Ok(());
        }

        // メッセージのメンションチャンネルを取得
        let re = Regex::new(r"<#(\d+)>").unwrap();
        let caps = re
            .captures(&message.content)
            .and_then(|caps| caps.get(0))
            .context("チャンネルメンションの取得に失敗")?;
        let mention_channel_id = ChannelId::from_str(caps.as_str())
            .context("メンションされたチャンネルIDの取得に失敗")?;

        // リアクションを追加した人がボイスチャンネルにいるか確認
        let guild_id = reaction.guild_id.context("サーバーの取得に失敗")?;
        let guild = guild_id
            .to_guild_cached(&ctx)
            .context("サーバーの取得に失敗")?;
        let voice_state = guild
            .voice_states
            .get(&user_id)
            .context("ボイスチャンネルに参加していません")?;
        let voice_channel_id = voice_state
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

        // リアクションをした人全員をボイスチャンネルに移動
        for user in &reaction_users {
            // 通話状態を取得
            let voice_state = match guild.voice_states.get(&user.id) {
                Some(voice_state) => voice_state,
                None => continue,
            };

            // 通話中のチャンネルIDを取得
            let channel_id = match voice_state.channel_id {
                Some(channel_id) => channel_id,
                None => continue,
            };

            // 同じチャンネルにいなければ無視
            if channel_id != voice_channel_id {
                continue;
            }

            // メンバーを取得
            let member = match guild
                .member(&ctx, user.id)
                .await
                .context("メンバーの取得に失敗")
            {
                Ok(member) => member,
                Err(_) => continue,
            };

            // リアクションを追加した人がボイスチャンネルにいる場合は移動
            let _ = member.move_to_voice_channel(&ctx, mention_channel_id).await;
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
                    reaction_users.len() - 1,
                    mention_channel_id.mention()
                ));
                message.embed(|embed| {
                    embed.title("移動したメンバー");
                    embed.description(
                        reaction_users
                            .iter()
                            .map(|user| user.mention().to_string())
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
        warn!("Bot準備完了: {}", data_about_bot.user.tag());

        // コマンドを登録
        let result = Command::create_global_application_command(&ctx, |command| {
            command
                .name("move")
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
        .await;
        match result {
            Ok(command) => {
                self.move_command_id.lock().await.replace(command);
            }
            Err(why) => {
                error!("コマンドの登録に失敗: {:?}", why);
                return;
            }
        }
    }

    /// コマンドが実行されたときに呼ばれる
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // コマンドを取得
        let command_id = match self.move_command_id.lock().await.as_ref() {
            Some(command) => command.id,
            None => {
                error!("コマンドが登録されていません");
                return;
            }
        };

        // 不明なインタラクションは無視
        match interaction {
            Interaction::ApplicationCommand(interaction) if interaction.data.id == command_id => {
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
