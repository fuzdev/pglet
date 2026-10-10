-- **A false-rejection control — do not trim it.** `find_coercion_pathway` returns
-- `COERCION_PATH_COERCEVIAIO` whenever the context is at least assignment and the
-- *target* is `TYPCATEGORY_STRING`, so every type assigns into `text` / `varchar` with
-- no `pg_cast` row involved — including the four here that have none. A gate built from
-- `pg_cast` alone (the obvious reading of "explicit-only pairs") refuses all of them.
--
-- The mirror rule needs `COERCION_EXPLICIT`, which is why the reverse direction is
-- `42804` (`assignment_cast_typed_text_into_int_loud.sql`).
-- @setup
CREATE TABLE acs (id INT, t1 TEXT, t2 TEXT, t3 TEXT, t4 VARCHAR(64));
-- @test
INSERT INTO acs (id, t1, t2, t3, t4)
VALUES (1, TRUE, '{"k":1}'::jsonb, '11111111-1111-1111-1111-111111111111'::uuid, 7)
RETURNING id, t1, t2, t3, t4;
