-- A target alias hides the target's **table name** from `RETURNING`: over
-- `UPDATE rat_t o …`, `RETURNING rat_t.v` is `42P01` on 18.6 (`invalid reference to
-- FROM-clause entry for table "rat_t"`) and `42P01` here.
--
-- The effective name is the *whole* qualifier surface — the alias when one was written,
-- the table name otherwise — and conflating the two is what let this statement resolve
-- and answer a row. The source-side mirror is
-- `returning_from_source_name_hidden_by_alias.sql`, and the accepting spelling of this
-- one (`RETURNING o.v`) is in `returning_from_alias_and_source_columns.sql`.
--
-- The `FROM` is load-bearing only in that it makes this the exact statement shape the
-- widened namespace builds: both relations are aliased, so *neither* table name is in
-- scope, and a build that fell back to catalog lookup for an unmatched qualifier
-- resolves both.
-- @setup
CREATE TABLE rat_t (id INT, v TEXT);
CREATE TABLE rat_s (id INT, sv TEXT);
INSERT INTO rat_t VALUES (1, 'TARGET');
INSERT INTO rat_s VALUES (1, 'SOURCE');

-- @test
-- @expect-error: 42P01 — the target's alias hides its table name
UPDATE rat_t o SET v = 'hit' FROM rat_s s WHERE s.id = o.id RETURNING rat_t.v;
