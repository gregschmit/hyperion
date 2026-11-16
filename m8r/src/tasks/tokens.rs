use loco_rs::prelude::*;

use crate::{models::tokens, views};

pub struct Tokens;
#[async_trait]
impl Task for Tokens {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "tokens".to_string(),
            detail: "List the tokens for the API and the MCP endpoint: each one's name, when it \
                     was made, and what it covers."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let tokens = tokens::Model::list(&ctx.db).await?;
        if tokens.is_empty() {
            println!("No tokens. Make one: task token name:duet scope:watchlist_items");
        }
        for token in tokens {
            let made = views::time(&token.created_at);
            println!("{}\t{made}\t{}", token.name, token.scope);
        }
        Ok(())
    }
}
