use loco_rs::prelude::*;

use crate::initializers::access;

pub struct Link;
#[async_trait]
impl Task for Link {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "link".to_string(),
            detail: "Print an access link. It works any number of times for 8 hours.\n\
                     hours:72 makes it last 72 hours, and once:true makes it work one time."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let seconds = match vars.cli_arg("hours") {
            Ok(hours) => {
                let hours: u64 = hours
                    .parse()
                    .ok()
                    .filter(|hours| *hours > 0)
                    .ok_or_else(|| Error::string("hours must be a number, such as hours:72"))?;
                hours * 60 * 60
            }
            Err(_) => ctx.config.get_jwt_config()?.expiration,
        };
        let once = match vars.cli_arg("once") {
            Ok("true") => true,
            Ok("false") | Err(_) => false,
            Ok(_) => return Err(Error::string("once must be once:true or once:false")),
        };
        println!("{}", access::link(ctx, seconds, once)?);
        Ok(())
    }
}
