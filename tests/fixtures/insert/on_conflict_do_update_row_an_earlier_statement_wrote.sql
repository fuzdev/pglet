-- The refusal is this statement's alone. The harness runs `@setup` and `@test` in one
-- transaction block, so the row the `@test` meets was inserted by an earlier statement of
-- the same block — and 18.6 updates it (its command id is older than the current one, so
-- locking it answers `TM_Ok`). A refusal keyed on the row's writing *transaction* refuses
-- this statement.
-- @profile-lock
-- @setup
CREATE TABLE ocew (id INT PRIMARY KEY, v TEXT);
INSERT INTO ocew VALUES (1, 'a');
INSERT INTO ocew VALUES (1, 'b') ON CONFLICT (id) DO UPDATE SET v = excluded.v;

-- @test
INSERT INTO ocew VALUES (1, 'c') ON CONFLICT (id) DO UPDATE SET v = ocew.v || excluded.v
RETURNING *;
