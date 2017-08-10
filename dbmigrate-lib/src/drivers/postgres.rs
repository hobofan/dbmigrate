use postgres_client::{Connection, TlsMode};
use postgres_client::tls::native_tls::NativeTls;
use url::Url;

use super::Driver;
use errors::{Result, ResultExt};

const SSLMODE: &'static str = "sslmode";


#[derive(Debug)]
pub struct Postgres {
    conn: Connection
}

impl Postgres {
    pub fn new(url: &str) -> Result<Postgres> {
        let conn = mk_connection(url)?;
        let pg = Postgres { conn: conn };
        pg.ensure_migration_table_exists();
        Ok(pg)
    }

    pub fn schema() -> String {
        ::std::env::var("DBMIGRATE_SCHEMA").unwrap_or("public".to_owned())
    }
}

impl Driver for Postgres {
    fn ensure_migration_table_exists(&self) {
        self.conn.batch_execute(&format!("
            CREATE TABLE IF NOT EXISTS {0}.__dbmigrate_table(id INTEGER, current INTEGER);
            INSERT INTO {0}.__dbmigrate_table (id, current)
            SELECT 1, 0
            WHERE NOT EXISTS(SELECT * FROM {0}.__dbmigrate_table WHERE id = 1);
        ", &Self::schema())).unwrap();
    }

    fn remove_migration_table(&self) {
        self.conn.execute(&format!("DROP TABLE {0}.__dbmigrate_table;", &Self::schema()), &[]).unwrap();
    }

    fn get_current_number(&self) -> i32 {
        let stmt = self.conn.prepare(&format!("
            SELECT current FROM {0}.__dbmigrate_table WHERE id = 1;
        ", &Self::schema())).unwrap();
        let results = stmt.query(&[]).unwrap();

        results.get(0).get("current")
    }

    fn set_current_number(&self, number: i32) {
        let stmt = self.conn.prepare(&format!(
            "UPDATE {0}.__dbmigrate_table SET current = $1 WHERE id = 1;",
        &Self::schema())).unwrap();
        stmt.execute(&[&number]).unwrap();
    }

    fn migrate(&self, migration: String, number: i32) -> Result<()> {
        self.conn.batch_execute(&migration).chain_err(|| "Migration failed")?;
        self.set_current_number(number);

        Ok(())
    }
}

// rust-postgres doesn't automatically support SSL from the url
// (https://github.com/sfackler/rust-postgres/issues/166)
// So we need to parse the url manually to check if we have some sslmode in it
// and create a connection with the correct one
fn mk_connection(url: &str) -> Result<Connection> {
    let negotiator = NativeTls::new().unwrap();
    let url = Url::parse(url).unwrap();
    let sslmode = url.query_pairs()
        .find(|&(ref k, _)| k == SSLMODE)
        .map_or(
            TlsMode::None,
            |(_, v)| match v.as_ref() {
                "allow" | "prefer" => TlsMode::Prefer(&negotiator),
                "require" => TlsMode::Require(&negotiator),
                // No support for certificate verification yet.
                "verify-ca" | "verify-full" => unimplemented!(),
                _ => TlsMode::None
            }
        );

    Connection::connect(without_sslmode(&url).as_ref(), sslmode).map_err(From::from)
}

fn without_sslmode(url: &Url) -> String {
    let pairs = url.query_pairs()
        .filter(|&(ref k, _)| k != SSLMODE);

    let mut cloned_url = url.clone();
    cloned_url.query_pairs_mut().clear();
    for (name, value) in pairs {
        cloned_url.query_pairs_mut().append_pair(name.as_ref(), value.as_ref());
    }

    cloned_url.as_str().to_owned()
}
