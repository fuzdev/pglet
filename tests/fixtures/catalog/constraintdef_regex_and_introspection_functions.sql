-- `pg_get_constraintdef` over a stored `~` / `!~`, `quote_ident` and `has_schema_privilege`:
-- each unknown literal reads back cast to the `text` its position resolved to.
-- @setup
CREATE TABLE cx (a int, s text, CHECK (quote_ident(s) <> ''), CHECK (has_schema_privilege('public', 'USAGE') OR a > 0), CHECK (s ~ '^x' OR s !~ '^y'));
-- @test
SELECT c.conname, pg_get_constraintdef(c.oid) AS def
FROM pg_constraint c JOIN pg_class r ON r.oid = c.conrelid
WHERE r.relname = 'cx' ORDER BY c.conname;
