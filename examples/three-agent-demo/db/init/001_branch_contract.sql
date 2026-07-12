CREATE TABLE schema_migrations (
  migration_id text PRIMARY KEY,
  agent text NOT NULL,
  payload text NOT NULL
);

CREATE TABLE seeded_accounts (
  account_id integer PRIMARY KEY,
  owner text NOT NULL
);

INSERT INTO schema_migrations VALUES
  ('202607090001', 'base', 'base checkout; demo.sh replaces this per stackstead');
INSERT INTO seeded_accounts VALUES (1, 'base');
