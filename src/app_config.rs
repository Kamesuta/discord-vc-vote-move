use anyhow::{Context as _, Result};
use config::Config;
use serenity::model::prelude::ChannelId;

#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct DiscordConfig {
    /// 投票の制限時間
    pub move_timeout_minutes: u64,
    /// 最初の1人が移動してから他の人が移動するまでの時間
    pub move_wait_seconds: u64,
    /// VC作成チャンネル
    pub vc_create_channel: ChannelId,
    /// Botが動作するカテゴリID
    pub vc_category: ChannelId,
    /// 無視するチャンネルID
    pub vc_ignored_channels: Vec<ChannelId>,
}

/// アプリケーションの設定
#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct AppConfig {
    /// Discordの設定
    pub discord: DiscordConfig,
}

impl AppConfig {
    /// 設定を読み込む
    pub fn load_config(basedir: &str) -> Result<AppConfig> {
        // 設定ファイルのパス
        let path = format!("{}/config.toml", basedir);
        // 設定ファイルを読み込む
        let config = Config::builder()
            // Add in `./Settings.toml`
            .add_source(config::File::with_name(&path))
            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `APP_DEBUG=1 ./target/app` would set the `debug` key
            .add_source(config::Environment::with_prefix("APP"))
            .build()?;
        // 設定ファイルをパース
        let app_config = config
            .try_deserialize::<AppConfig>()
            .context("設定ファイルの読み込みに失敗")?;
        Ok(app_config)
    }
}
