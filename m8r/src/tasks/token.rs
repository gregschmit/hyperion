use loco_rs::prelude::*;

use crate::models::tokens;

pub struct Token;
#[async_trait]
impl Task for Token {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "token".to_string(),
            detail: format!(
                "Make a token for the API and the MCP endpoint, and print it. It shows this one \
                 time.\nname:duet says what will use it. scope:watchlist_items says what it \
                 covers: {}, or one or more of {}, with commas between them.",
                tokens::ALL,
                tokens::RESOURCES.join(", ")
            ),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let name = vars
            .cli_arg("name")
            .map_err(|_| Error::string("say what will use the token, such as name:duet"))?;
        let scope = vars.cli_arg("scope").map_err(|_| {
            Error::string("say what the token covers, such as scope:watchlist_items or scope:all")
        })?;
        let token = tokens::Model::create(&ctx.db, name, scope)
            .await?
            .map_err(|problem| Error::string(&problem))?;
        println!("{token}");
        Ok(())
    }
}
