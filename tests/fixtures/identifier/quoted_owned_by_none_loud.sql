-- `NONE` in `ALTER SEQUENCE … OWNED BY NONE` is a **grammar keyword**, not a name, so a
-- quoted `"NONE"` is an ordinary one-part name and the clause is malformed: `42601` on
-- both engines. pglet used to compare the parsed word case-insensitively without
-- regard to quoting, so `"NONE"` silently detached the ownership edge — a catalog
-- write from a statement Postgres refuses to parse.
-- The accept side is `sequences/alter_owned_by_none_detaches.sql` and its neighbours,
-- which spell it unquoted.
-- @setup
CREATE SEQUENCE qobn;

-- @test
ALTER SEQUENCE qobn OWNED BY "NONE";
-- @expect-error: `NONE` is a keyword, so the quoted spelling is not it (42601)
-- @no-error-fields: 18.6 sends no `P` on this route at all; pglet points at the terminator
