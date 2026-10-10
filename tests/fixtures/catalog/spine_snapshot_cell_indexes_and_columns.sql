-- The spine shapes read through `pg_indexes` — where the partial-index predicate and
-- the quoted key column live. (The `@test` reads **only** `pg_indexes`: the header used
-- to claim `information_schema.columns` too, which is
-- `column_default_literal_takes_the_column_type_label.sql`'s subject, and the enum
-- `DEFAULT` seeded here is setup for the index rows rather than something this fixture
-- asserts.) The `position` key column is the second shape: it is a keyword on both
-- engines, so 18.6 prints `(parent_id, "position")` and pglet's own
-- `pg_get_constraintdef` already did, while `pg_indexes.indexdef` was the one name
-- position that skipped `quote_ident`.
-- @setup
CREATE TYPE spi_visibility AS ENUM ('private', 'public');
CREATE TABLE spi_cell (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  visibility  spi_visibility NOT NULL DEFAULT 'private',
  moderation  TEXT,
  root_id     UUID
);
CREATE INDEX idx_spi_cell_moderation ON spi_cell(root_id) WHERE moderation = 'pending';
CREATE TABLE spi_item (
  parent_id UUID NOT NULL,
  position  TEXT NOT NULL,
  PRIMARY KEY (parent_id, position)
);
CREATE INDEX idx_spi_item_parent_position ON spi_item(parent_id, position);
-- @param $1 text public
-- @test
SELECT tablename::text AS tablename, indexname::text AS indexname, indexdef
FROM pg_indexes WHERE schemaname::text = $1
ORDER BY tablename::text ASC, indexname::text ASC
