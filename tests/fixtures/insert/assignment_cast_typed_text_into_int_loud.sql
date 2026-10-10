-- The class is **wider than `pg_cast`'s explicit-only rows**: `text → integer` has no
-- `pg_cast` entry at all, and a pair with no entry is refused in assignment context just
-- as an `'e'` pair is (`find_coercion_pathway` allows I/O coercion only into a
-- string-category *target*). pglet ran the text through its input function instead, so
-- this stored `5` — and its unparseable sibling raised `22P02` at write time rather than
-- `42804` at parse.
--
-- The **cast** is the whole fixture: the bare `'5'` spelling of the same statement is
-- Postgres's `unknown` and is ordinary SQL on both engines
-- (`assignment_cast_unknown_literal_resolves_to_the_column.sql`).
-- @setup
CREATE TABLE act (id INT, i INT);

-- @expect-error: column "i" is of type integer but expression is of type text (42804)
-- @test
INSERT INTO act (id, i) VALUES (1, '5'::text);
