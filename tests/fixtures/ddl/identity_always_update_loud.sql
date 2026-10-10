-- An UPDATE setting a GENERATED ALWAYS identity column to a non-DEFAULT value is
-- rejected — Postgres SQLSTATE 428C9 (generated_always): `column "id" can only be
-- updated to DEFAULT`. (UPDATE has no OVERRIDING clause, so there is no override.)
-- @expect-error: a GENERATED ALWAYS column can only be updated to DEFAULT
-- @setup
CREATE TABLE t (id int GENERATED ALWAYS AS IDENTITY, name text);
INSERT INTO t (name) VALUES ('a');
-- @test
UPDATE t SET id = 5 WHERE name = 'a';
