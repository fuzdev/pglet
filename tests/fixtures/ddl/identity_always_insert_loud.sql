-- An explicit (non-DEFAULT) value for a GENERATED ALWAYS identity column, without
-- OVERRIDING SYSTEM VALUE, is rejected — Postgres SQLSTATE 428C9
-- (generated_always): `cannot insert a non-DEFAULT value into column "id"`.
-- @expect-error: cannot insert a non-DEFAULT value into a GENERATED ALWAYS column
-- @setup
CREATE TABLE t (id int GENERATED ALWAYS AS IDENTITY, name text);
-- @test
INSERT INTO t (id, name) VALUES (5, 'x');
