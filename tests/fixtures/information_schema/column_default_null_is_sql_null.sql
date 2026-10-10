-- `DEFAULT NULL` reports **SQL NULL** in `information_schema.columns.column_default`,
-- not the four characters `NULL`.
--
-- 18.6 stores no `pg_attrdef` row for a NULL-constant default, so the view's
-- `column_default` is NULL exactly as it is for a column with no DEFAULT clause
-- (measured: 0 rows in `pg_attrdef`, `column_default IS NULL` true, and `pg_dump` emits
-- no `DEFAULT`). pglet rendered the string `NULL`, so a migration tool testing
-- `column_default IS NOT NULL` saw a default that does not exist.
--
-- The existing `column_defaults.sql` fixture filters `column_default IS NOT NULL`,
-- which is exactly the polarity that hides this — hence the explicit
-- `default_is_sql_null` projection and no filter here. The `n` column is the control:
-- a real default still reports its text.
-- @setup
CREATE TABLE cdn_t (
  id INT,
  c INT DEFAULT NULL,
  n INT DEFAULT 1
);
-- @test
SELECT column_name::text AS column_name,
       column_default::text AS column_default,
       (column_default IS NULL) AS default_is_sql_null
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'cdn_t'
ORDER BY ordinal_position;
