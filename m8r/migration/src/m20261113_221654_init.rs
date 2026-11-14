use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let blank = String::new;

        create_table(
            m,
            "configurations",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::String),
                ("active", ColType::BooleanWithDefault(false)),
                ("system_message", ColType::TextWithDefault(blank())),
                ("triage_prompt", ColType::TextWithDefault(blank())),
                ("review_prompt", ColType::TextWithDefault(blank())),
                ("develop_prompt", ColType::TextWithDefault(blank())),
            ],
            &[],
        )
        .await?;
        // At most one configuration is active. A partial index, which sea-query can't build.
        m.get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX idx_configurations_one_active ON configurations (active) \
                 WHERE active",
            )
            .await?;

        create_table(
            m,
            "schedules",
            &[
                ("id", ColType::PkAuto),
                ("operation", ColType::String),
                ("repo", ColType::String),
                ("cron", ColType::String),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "items",
            &[
                ("id", ColType::PkAuto),
                ("url", ColType::StringUniq),
                ("priority", ColType::StringWithDefault("medium".to_string())),
                ("note", ColType::StringWithDefault(blank())),
                // A copy of the forge's title, for the pages.
                ("title", ColType::StringWithDefault(blank())),
                ("checked_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "runs",
            &[
                ("id", ColType::PkAuto),
                ("operation", ColType::String),
                ("repo", ColType::String),
                ("status", ColType::StringWithDefault("queued".to_string())),
                ("prompt", ColType::TextWithDefault(blank())),
                ("output", ColType::TextWithDefault(blank())),
                ("error", ColType::TextWithDefault(blank())),
                ("started_at", ColType::TimestampWithTimeZoneNull),
                ("finished_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[],
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_runs_status")
                .table(Alias::new("runs"))
                .col(Alias::new("status"))
                .to_owned(),
        )
        .await?;

        // `run_id` references runs, ON DELETE CASCADE.
        create_table(
            m,
            "artifacts",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::String),
                // Blank when m8r hosts the artifact's files.
                ("url", ColType::StringWithDefault(blank())),
            ],
            &[("run", "")],
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_artifacts_run_name")
                .table(Alias::new("artifacts"))
                .col(Alias::new("run_id"))
                .col(Alias::new("name"))
                .unique()
                .to_owned(),
        )
        .await?;

        create_table(
            m,
            "used_links",
            &[
                ("id", ColType::PkAuto),
                ("token_id", ColType::StringUniq),
                ("expires_at", ColType::TimestampWithTimeZone),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "tool_errors",
            &[
                ("id", ColType::PkAuto),
                ("tool", ColType::String),
                // Blank when the error is about the tool itself, not one host's login.
                ("host", ColType::StringWithDefault(blank())),
                ("message", ColType::TextWithDefault(blank())),
            ],
            &[],
        )
        .await?;
        m.create_index(
            Index::create()
                .name("idx_tool_errors_tool_host")
                .table(Alias::new("tool_errors"))
                .col(Alias::new("tool"))
                .col(Alias::new("host"))
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "tool_errors",
            "used_links",
            "artifacts",
            "runs",
            "items",
            "schedules",
            "configurations",
        ] {
            drop_table(m, table).await?;
        }
        Ok(())
    }
}
