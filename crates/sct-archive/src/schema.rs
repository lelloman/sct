//! Creation descriptions for private offline SQLite files, not a format validator.
use crate::Result;
use simple_server::database::sqlite::schema::*;
use std::borrow::Cow;

fn id(name: &'static str) -> Identifier<'static> {
    Identifier::new(name).expect("constant archive identifier")
}
fn ids(names: &[&'static str]) -> Vec<Identifier<'static>> {
    names.iter().map(|name| id(name)).collect()
}
fn table(
    name: &'static str,
    columns: &[(&'static str, &'static str, bool)],
    primary: &[&'static str],
) -> TableSpec<'static> {
    TableSpec {
        name: id(name),
        columns: Cow::Owned(
            columns
                .iter()
                .map(|(name, ty, not_null)| ColumnSpec {
                    name: id(name),
                    declared_type: Cow::Borrowed(ty),
                    not_null: *not_null,
                    default: None,
                })
                .collect(),
        ),
        primary_key: Cow::Owned(ids(primary)),
        unique_constraints: Cow::Owned(Vec::new()),
        foreign_keys: Cow::Owned(Vec::new()),
        indexes: Cow::Owned(Vec::new()),
        unsupported: Cow::Borrowed(&[]),
    }
}
fn unique(table: &mut TableSpec<'static>, columns: &[&'static str]) {
    table.unique_constraints.to_mut().push(ids(columns));
}
fn foreign(
    table: &mut TableSpec<'static>,
    columns: &[&'static str],
    parent: &'static str,
    targets: &[&'static str],
) {
    table.foreign_keys.to_mut().push(ForeignKeySpec {
        columns: Cow::Owned(ids(columns)),
        parent_table: id(parent),
        parent_columns: Cow::Owned(ids(targets)),
        on_update: ForeignKeyAction::NoAction,
        on_delete: ForeignKeyAction::NoAction,
    });
}
fn index(table: &mut TableSpec<'static>, name: &'static str, columns: &[&'static str]) {
    table.indexes.to_mut().push(IndexSpec {
        name: id(name),
        terms: Cow::Owned(
            columns
                .iter()
                .map(|name| IndexTerm {
                    column: id(name),
                    collation: id("BINARY"),
                    descending: false,
                })
                .collect(),
        ),
        unique: false,
        predicate: None,
    });
}
fn plan(
    tables: Vec<TableSpec<'static>>,
    options: Vec<TableCreationOptions<'static>>,
) -> Result<CreationPlan> {
    let snapshot = ExtendedSchemaSnapshot::new(SchemaSnapshot {
        namespace: Cow::Borrowed("sct/offline-archive"),
        database: id("main"),
        version: 1,
        tables: Cow::Owned(tables),
    });
    Ok(create_extended_plan_with_options(
        &snapshot,
        CreationMode::Create,
        &CreationOptions {
            tables: Cow::Owned(options),
            virtual_tables: Cow::Borrowed(&[]),
        },
    )?)
}

pub(crate) fn index_plan() -> Result<CreationPlan> {
    let meta = table(
        "meta",
        &[("descriptor", "TEXT", true), ("snapshot", "TEXT", true)],
        &[],
    );
    let mut discs = table(
        "discs",
        &[
            ("id", "TEXT", false),
            ("number", "INTEGER", true),
            ("set_sequence", "INTEGER", true),
        ],
        &["id"],
    );
    unique(&mut discs, &["number"]);
    let entries = table(
        "entries",
        &[
            ("path", "TEXT", false),
            ("kind", "TEXT", true),
            ("root", "TEXT", false),
            ("version", "TEXT", false),
            ("length", "INTEGER", true),
        ],
        &["path"],
    );
    let mut objects = table(
        "objects",
        &[
            ("root", "TEXT", false),
            ("digest", "TEXT", false),
            ("length", "INTEGER", true),
        ],
        &["root", "digest"],
    );
    unique(&mut objects, &["root", "digest", "length"]);
    let mut extents = table(
        "extents",
        &[
            ("path", "TEXT", false),
            ("offset", "INTEGER", false),
            ("root", "TEXT", false),
            ("digest", "TEXT", false),
            ("length", "INTEGER", false),
        ],
        &["path", "offset"],
    );
    foreign(&mut extents, &["path"], "entries", &["path"]);
    foreign(
        &mut extents,
        &["root", "digest", "length"],
        "objects",
        &["root", "digest", "length"],
    );
    let mut placements = table(
        "placements",
        &[
            ("root", "TEXT", false),
            ("digest", "TEXT", false),
            ("length", "INTEGER", false),
            ("disc", "TEXT", false),
        ],
        &["root", "digest", "disc"],
    );
    foreign(&mut placements, &["disc"], "discs", &["id"]);
    foreign(
        &mut placements,
        &["root", "digest", "length"],
        "objects",
        &["root", "digest", "length"],
    );
    index(
        &mut placements,
        "placement_disc",
        &["disc", "root", "digest"],
    );
    plan(
        vec![meta, discs, entries, objects, extents, placements],
        vec![],
    )
}

pub(crate) fn catalog_header_plan() -> Result<CreationPlan> {
    plan(
        vec![table("catalog_meta", &[("format", "INTEGER", true)], &[])],
        vec![TableCreationOptions {
            table: id("catalog_meta"),
            strict: false,
            checks: Cow::Owned(vec![CheckConstraint {
                name: None,
                column: Some(id("format")),
                expression: SqlExpression::trusted("format=1")?,
            }]),
        }],
    )
}
pub(crate) fn catalog_body_plan() -> Result<CreationPlan> {
    let mut sets = table(
        "catalog_sets",
        &[
            ("collection", "TEXT", true),
            ("id", "TEXT", true),
            ("sequence", "INTEGER", true),
            ("index_hash", "TEXT", true),
            ("bundle_hash", "TEXT", true),
            ("descriptor", "TEXT", true),
        ],
        &["collection", "id"],
    );
    unique(&mut sets, &["collection", "sequence"]);
    let mut discs = table(
        "catalog_discs",
        &[
            ("collection", "TEXT", true),
            ("id", "TEXT", true),
            ("number", "INTEGER", true),
            ("set_sequence", "INTEGER", true),
        ],
        &["collection", "id"],
    );
    unique(&mut discs, &["collection", "number"]);
    let mut scans = table(
        "catalog_scans",
        &[
            ("collection", "TEXT", true),
            ("disc", "TEXT", true),
            ("descriptor", "TEXT", true),
        ],
        &["collection", "disc"],
    );
    foreign(
        &mut scans,
        &["collection", "disc"],
        "catalog_discs",
        &["collection", "id"],
    );
    let mut records = table(
        "catalog_records",
        &[
            ("collection", "TEXT", true),
            ("set_id", "TEXT", true),
            ("position", "INTEGER", true),
            ("line", "BLOB", true),
        ],
        &["collection", "set_id", "position"],
    );
    foreign(
        &mut records,
        &["collection", "set_id"],
        "catalog_sets",
        &["collection", "id"],
    );
    let mut placements = table(
        "catalog_placements",
        &[
            ("collection", "TEXT", true),
            ("set_id", "TEXT", true),
            ("disc", "TEXT", true),
            ("root", "TEXT", true),
            ("digest", "TEXT", true),
            ("length", "INTEGER", true),
        ],
        &["collection", "set_id", "root", "digest", "disc"],
    );
    foreign(
        &mut placements,
        &["collection", "set_id"],
        "catalog_sets",
        &["collection", "id"],
    );
    foreign(
        &mut placements,
        &["collection", "disc"],
        "catalog_discs",
        &["collection", "id"],
    );
    index(
        &mut placements,
        "catalog_placement_disc",
        &["collection", "disc", "root", "digest", "length"],
    );
    index(
        &mut placements,
        "catalog_object_length",
        &["collection", "root", "digest", "length"],
    );
    plan(vec![sets, discs, scans, records, placements], vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Connection, Row, SqliteConnection};

    async fn observe(connection: &mut SqliteConnection) -> Result<Vec<String>> {
        let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .fetch_all(&mut *connection).await?;
        let mut result = Vec::new();
        for table in tables {
            result.push(format!("table:{table}"));
            for row in sqlx::query(
                "SELECT name,type,[notnull],dflt_value,pk FROM pragma_table_info(?) ORDER BY cid",
            )
            .bind(&table)
            .fetch_all(&mut *connection)
            .await?
            {
                result.push(format!(
                    "column:{:?}",
                    (
                        row.get::<String, _>(0),
                        row.get::<String, _>(1),
                        row.get::<i64, _>(2),
                        row.get::<Option<String>, _>(3),
                        row.get::<i64, _>(4)
                    )
                ));
            }
            for row in sqlx::query("SELECT seq,[table],[from],[to],on_update,on_delete,[match] FROM pragma_foreign_key_list(?) ORDER BY [table],[from],seq")
                .bind(&table).fetch_all(&mut *connection).await? {
                result.push(format!("fk:{:?}",(row.get::<i64,_>(0),row.get::<String,_>(1),row.get::<String,_>(2),row.get::<String,_>(3),row.get::<String,_>(4),row.get::<String,_>(5),row.get::<String,_>(6))));
            }
            let indexes = sqlx::query(
                "SELECT name,[unique],origin,partial FROM pragma_index_list(?) ORDER BY name",
            )
            .bind(&table)
            .fetch_all(&mut *connection)
            .await?;
            for row in indexes {
                let name = row.get::<String, _>(0);
                result.push(format!(
                    "index:{:?}",
                    (
                        &name,
                        row.get::<i64, _>(1),
                        row.get::<String, _>(2),
                        row.get::<i64, _>(3)
                    )
                ));
                for term in sqlx::query("SELECT seqno,cid,name,[desc],coll,[key] FROM pragma_index_xinfo(?) ORDER BY seqno")
                    .bind(name).fetch_all(&mut *connection).await? {
                    result.push(format!("term:{:?}",(term.get::<i64,_>(0),term.get::<i64,_>(1),term.get::<Option<String>,_>(2),term.get::<i64,_>(3),term.get::<String,_>(4),term.get::<i64,_>(5))));
                }
            }
        }
        Ok(result)
    }
    async fn create(connection: &mut SqliteConnection, catalog: bool) -> Result<()> {
        let mut tx = connection.begin().await?;
        let plan = if catalog {
            catalog_header_plan()?
        } else {
            index_plan()?
        };
        for statement in plan.statements {
            sqlx::raw_sql(&statement).execute(&mut *tx).await?;
        }
        if catalog {
            sqlx::query("INSERT INTO catalog_meta VALUES(1)")
                .execute(&mut *tx)
                .await?;
            for statement in catalog_body_plan()?.statements {
                sqlx::raw_sql(&statement).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn creation_preserves_real_legacy_metadata_and_constraints() -> Result<()> {
        for (catalog, legacy_sql) in [
            (false, include_str!("index_legacy_schema_test.sql")),
            (true, include_str!("catalog_legacy_schema_test.sql")),
        ] {
            let mut old = SqliteConnection::connect("sqlite::memory:").await?;
            let mut new = SqliteConnection::connect("sqlite::memory:").await?;
            for connection in [&mut old, &mut new] {
                sqlx::query("PRAGMA foreign_keys=ON")
                    .execute(connection)
                    .await?;
            }
            sqlx::raw_sql(legacy_sql).execute(&mut old).await?;
            create(&mut new, catalog).await?;
            assert_eq!(observe(&mut old).await?, observe(&mut new).await?);
            for connection in [&mut old, &mut new] {
                if catalog {
                    assert_eq!(
                        sqlx::query_scalar::<_, i64>("SELECT format FROM catalog_meta")
                            .fetch_one(&mut *connection)
                            .await?,
                        1
                    );
                    assert!(
                        sqlx::query("UPDATE catalog_meta SET format=2")
                            .execute(&mut *connection)
                            .await
                            .is_err()
                    );
                    assert!(
                        sqlx::query("INSERT INTO catalog_meta VALUES(NULL)")
                            .execute(&mut *connection)
                            .await
                            .is_err()
                    );
                    assert!(
                        sqlx::query("INSERT INTO catalog_scans VALUES('c','unknown','d')")
                            .execute(&mut *connection)
                            .await
                            .is_err()
                    );
                    sqlx::query("INSERT INTO catalog_discs VALUES('c','d',1,1)")
                        .execute(&mut *connection)
                        .await?;
                    assert!(
                        sqlx::query("INSERT INTO catalog_discs VALUES('c','other',1,1)")
                            .execute(&mut *connection)
                            .await
                            .is_err()
                    );
                } else {
                    sqlx::query("INSERT INTO discs VALUES('d',1,1)")
                        .execute(&mut *connection)
                        .await?;
                    assert!(
                        sqlx::query("INSERT INTO discs VALUES('other',1,1)")
                            .execute(&mut *connection)
                            .await
                            .is_err()
                    );
                    // Preserve nullable TEXT primary keys; creation must not strengthen them.
                    sqlx::query("INSERT INTO entries VALUES(NULL,'file',NULL,NULL,0)")
                        .execute(&mut *connection)
                        .await?;
                    assert!(
                        sqlx::query("INSERT INTO extents VALUES('missing',0,'r','h',1)")
                            .execute(&mut *connection)
                            .await
                            .is_err()
                    );
                }
            }
            assert!(create(&mut new, catalog).await.is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn catalog_creation_rolls_back_all_phases_and_marker() -> Result<()> {
        let mut connection = SqliteConnection::connect("sqlite::memory:").await?;
        sqlx::query("PRAGMA user_version=81")
            .execute(&mut connection)
            .await?;
        let mut tx = connection.begin().await?;
        for statement in catalog_header_plan()?.statements {
            sqlx::raw_sql(&statement).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO catalog_meta VALUES(1)")
            .execute(&mut *tx)
            .await?;
        for statement in catalog_body_plan()?.statements {
            sqlx::raw_sql(&statement).execute(&mut *tx).await?;
        }
        assert!(
            sqlx::query("INSERT INTO catalog_meta VALUES(2)")
                .execute(&mut *tx)
                .await
                .is_err()
        );
        tx.rollback().await?;
        assert!(observe(&mut connection).await?.is_empty());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                .fetch_one(&mut connection)
                .await?,
            81
        );
        Ok(())
    }

    #[tokio::test]
    async fn legacy_catalog_reopens_without_recreation_or_new_shape_policy() -> Result<()> {
        let path = std::env::temp_dir().join(format!("sct-07e-{}.sqlite", uuid::Uuid::new_v4()));
        std::fs::File::create(&path)?;
        let mut connection = crate::sqlite_connection::connect_archive(&path).await?;
        sqlx::raw_sql(include_str!("catalog_legacy_schema_test.sql"))
            .execute(&mut connection)
            .await?;
        sqlx::query("INSERT INTO catalog_discs VALUES('c','d',1,1)")
            .execute(&mut connection)
            .await?;
        // Reopen historically checks marker alone, not every schema property.
        sqlx::query("DROP INDEX catalog_object_length")
            .execute(&mut connection)
            .await?;
        connection.close().await?;
        drop(crate::ArchiveCatalog::open(&path).await?);
        let mut connection = crate::sqlite_connection::connect_archive(&path).await?;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog_discs")
                .fetch_one(&mut connection)
                .await?,
            1
        );
        sqlx::query("PRAGMA ignore_check_constraints=ON")
            .execute(&mut connection)
            .await?;
        sqlx::query("UPDATE catalog_meta SET format=2")
            .execute(&mut connection)
            .await?;
        connection.close().await?;
        assert!(crate::ArchiveCatalog::open(&path).await.is_err());
        std::fs::remove_file(path)?;
        Ok(())
    }
}
