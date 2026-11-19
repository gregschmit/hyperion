use loco_rs::schema::drop_table;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// All of it happens, or none of it: SQLite takes a change to the schema back too. So a
    /// step that fails leaves the database as the m8r that's running knows it.
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        // A schedule's prompt is all that a run gets, so m8r keeps no text for every prompt,
        // and needs none to run. Where a schedule's prompt named the configuration's, it gets
        // what the active one held, so a run gets what it got before. That was plain text:
        // `raw` keeps it so, where it has the marks of a template. With none active, it was
        // blank.
        db.execute_unprepared(
            "UPDATE schedules
                SET prompt = replace(replace(schedules.prompt, '{{ config_prompt }}', held.text),
                                     '{{config_prompt}}', held.text)
               FROM (SELECT coalesce(max(CASE
                              WHEN instr(active.text, '{{') + instr(active.text, '{%')
                                   + instr(active.text, '{#') > 0
                              THEN '{% raw %}' || active.text || '{% endraw %}'
                              ELSE active.text END), '') AS text
                       FROM (SELECT rtrim(configurations.prompt, char(9, 10, 13, 32)) AS text
                               FROM configurations WHERE configurations.active) AS active) AS held
              WHERE schedules.prompt LIKE '%config_prompt%'",
        )
        .await?;
        drop_table(m, "configurations").await?;
        // No tick asks for a configuration any more, so nothing else would clear its error.
        db.execute_unprepared(
            "UPDATE errors SET cleared_at = strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')
               WHERE errors.cleared_at IS NULL AND errors.subject = 'configuration'",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, _m: &SchemaManager) -> Result<(), DbErr> {
        // The configurations are gone.
        Err(DbErr::Migration(
            "This migration can't be taken back. Restore the database file from before it."
                .to_string(),
        ))
    }
}
