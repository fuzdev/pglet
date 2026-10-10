-- The coincidence the quoted-schema fix had to preserve, pinned in all three accepting
-- spellings. pglet does not downcase a name part, so `reduce_qualified_name`'s
-- case-insensitive `public` test is what reproduces Postgres's downcase for an
-- **unquoted** qualifier — while a **quoted** part is compared exactly, so `"public"`
-- still names the implicit schema and `"PUBLIC"` does not
-- (`nextval_literal_quoted_schema_loud.sql`). A repair that simply stopped folding
-- anything would satisfy that fixture and redden this one.
-- @setup
CREATE SEQUENCE s;
-- @test
SELECT nextval('public.s') AS bare_public, nextval('PUBLIC.s') AS upper_public, nextval('"public".s') AS quoted_public;
