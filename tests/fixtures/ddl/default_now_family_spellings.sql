-- The five `now`-family spellings are five different defaults, and
-- `information_schema.columns.column_default` reports each one as written.
--
-- Measured on 18.6: `now()`, `transaction_timestamp()`, `statement_timestamp()` and
-- `clock_timestamp()` render as their own calls, and `CURRENT_TIMESTAMP` renders as the
-- bare keyword (it is a SQL value function, not a `pg_proc` entry). `pg_dump` emits the
-- same five spellings.
--
-- pglet collapsed all five onto one `ColumnDefault::Now` and reported `now()` for every
-- one of them. That was not only a rendering wrong: the collapse also erased which of
-- them is volatile, so an `ADD COLUMN … DEFAULT clock_timestamp()` backfilled a single
-- instant into every row (see `add_column_clock_timestamp_backfills_per_row.sql`).
--
-- The values are not compared — they are clock reads — so this fixture reads the
-- catalog only.
-- @setup
CREATE TABLE dnf (
  a TIMESTAMPTZ DEFAULT now(),
  b TIMESTAMPTZ DEFAULT transaction_timestamp(),
  c TIMESTAMPTZ DEFAULT statement_timestamp(),
  d TIMESTAMPTZ DEFAULT clock_timestamp(),
  e TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
  f DATE DEFAULT CURRENT_DATE,
  g TIME DEFAULT CURRENT_TIME
);
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'dnf'
ORDER BY ordinal_position;
