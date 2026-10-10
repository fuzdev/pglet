-- A `FROM` source written with an alias hides its **table name** from `RETURNING`, so
-- `RETURNING <table>.col` is `42P01 invalid reference to FROM-clause entry` on 18.6 —
-- and `42P01` here.
--
-- The reject-side control for `returning_from_source_qualified_column.sql`: the same
-- statement, the same column, spelled through the table name instead of the alias.
-- A namespace registered under the *table* name rather than the alias answers a row
-- here and `42P01` on the accepting spelling, so the pair pins **which** name the
-- source went in under, which neither file can say alone. Its target-side mirror is
-- `update/returning_alias_hides_table_name.sql`.
--
-- Both engines reject, so the SQLSTATE is compared — and it must be `42P01` and not
-- `42703`: the distinction is what a client uses to tell a wrong relation name from a
-- wrong column name.
-- @setup
CREATE TABLE rha_t (id INT, v TEXT);
CREATE TABLE rha_s (id INT, sv TEXT, k INT);
INSERT INTO rha_t VALUES (1, 'TARGET');
INSERT INTO rha_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42P01 — the source's alias hides its table name
UPDATE rha_t SET v = 'hit' FROM rha_s s WHERE s.id = rha_t.id RETURNING rha_s.k;
