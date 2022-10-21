# DiscordVC移動Bot

みんなで一斉にVCを移動するBotです。

みんなで新しいVCへ移動するよ、と声をかけて移動したが、みんなは移動せず一人ぼっちに、という悲しい現実を解決します。

リアクションをつけた人のみが同時に指定されたVCへ移動できます。


## 使用方法

`/move 新しいVC名` とコマンドを入力します。  
![image](https://user-images.githubusercontent.com/16362824/197182568-94122894-88c9-480a-b3b8-3616ded7d156.png)

一緒に移動する人にリアクションをつけてもらいます。  
![image](https://user-images.githubusercontent.com/16362824/197182941-3694bdc6-83f7-424e-a132-6cca38e383f7.png)

最初にコマンドを打った人がリアクションをつけると、リアクションつけた人全員が新しいチャンネルへ移動します。  
![移動する様子](https://user-images.githubusercontent.com/16362824/197183316-aaf7bc8c-d7f4-442f-b36b-75f306b80b4d.gif)

## セットアップ

- 環境変数 `DISCORD_TOKEN` にBotのトークンを登録します
- `config.default.toml` をコピーし `config.toml` を作成します
- `config.toml` の設定を変更します
- `cargo run` で起動します

|設定名|説明|
|----|----|
|move_timeout_minutes|リアクション募集の時間制限(分)|
|move_wait_seconds|最初の1人が移動してから他の人が移動するまでのインターバル時間|
|vc_create_channel|VC作成チャンネル(AstroBotなどの、VCジェネレーターチャンネル)|
|vc_category|一時VCが作成されるカテゴリID|
|vc_ignored_channels|VC作成チャンネルや、参加した際に無視したいチャンネルを指定する|
