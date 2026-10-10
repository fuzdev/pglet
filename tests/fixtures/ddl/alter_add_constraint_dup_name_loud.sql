-- A named `ALTER TABLE … ADD CONSTRAINT <n> CHECK` whose name already belongs to a
-- constraint on the table is `42710` (duplicate_object) in Postgres — `constraint
-- "<n>" for relation "<t>" already exists` — not a silent second copy. pglet used to
-- add the duplicate silently; it now fails loud, matching PG.
-- @expect-error: duplicate constraint name (42710)
-- @setup
CREATE TABLE adc (a INT, b INT, CONSTRAINT adc_pos CHECK (a > 0));
-- @test
ALTER TABLE adc ADD CONSTRAINT adc_pos CHECK (b > 0);
