-- A 3-part qualified column reference (schema.table.column) fails loud — pglet has
-- a single implicit schema and doesn't model deeper qualification. Postgres resolves
-- it; pglet is stricter (rather than silently truncating to the last part).
-- @setup
CREATE TABLE q3 (id INT);
INSERT INTO q3 VALUES (1);
-- @expect-error: 3-part qualified column ref fails loud (pglet-stricter than PG)
-- @test
SELECT public.q3.id FROM q3;
