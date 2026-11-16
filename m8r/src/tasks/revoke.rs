use loco_rs::prelude::*;

use crate::models::tokens;

pub struct Revoke;
#[async_trait]
impl Task for Revoke {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "revoke".to_string(),
            detail: "End a token for the API and the MCP endpoint: name:duet. `task tokens` \
                     lists them."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let name = vars
            .cli_arg("name")
            .map_err(|_| Error::string("say which token, such as name:duet"))?;
        if !tokens::Model::revoke(&ctx.db, name).await? {
            return Err(Error::string(&format!("there's no token called {name}")));
        }
        println!("Revoked {name}.");
        Ok(())
    }
}
