use sqlx::{migrate::MigrationType, PgPool};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrations = false)]
async fn test_stair_migrations(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    for i in (0..MIGRATOR.iter().len()).step_by(2) {
        let (up, down) = if MIGRATOR.migrations[i].migration_type == MigrationType::ReversibleUp {
            (i, i + 1)
        } else {
            (i + 1, i)
        };

        for index in [up, down, up] {
            for migration in MIGRATOR.migrations[index]
                .sql
                .split(';')
                .filter(|f| !f.is_empty())
            {
                sqlx::query(migration).execute(&pool).await?;
            }
        }
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn test_downgrade(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    MIGRATOR.run(&pool).await?;
    for migration in MIGRATOR
        .iter()
        .filter(|f| f.migration_type == MigrationType::ReversibleDown)
        .rev()
    {
        MIGRATOR.undo(&pool, migration.version).await?;
    }
    Ok(())
}
