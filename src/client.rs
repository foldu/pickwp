use crate::{cli::Cmd, rpc};

pub async fn run(cmd: Cmd) -> Result<(), anyhow::Error> {
    let app_paths = crate::util::AppPaths::get().unwrap();
    let mut client = rpc::connect(app_paths.rt_dir).await?;
    let ctx = tarpc::context::current();
    match cmd {
        Cmd::Rescan => {
            client.scan(ctx).await?;
        }
        Cmd::Refresh => {
            client.refresh(ctx).await?;
        }
        Cmd::Current => {
            println!(
                "{}",
                serde_json::to_string_pretty(&client.get_wallpapers(ctx).await?).unwrap()
            );
        }
        Cmd::ToggleFreeze => {
            #[derive(serde::Serialize)]
            struct ToggleFreezeOutput {
                frozen: bool,
            }

            println!(
                "{}",
                serde_json::to_string_pretty(&ToggleFreezeOutput {
                    frozen: client.toggle_freeze(ctx).await?
                })
                .unwrap()
            );
        }
    }
    Ok(())
}
