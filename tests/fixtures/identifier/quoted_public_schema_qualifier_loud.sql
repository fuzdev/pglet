-- The **identifier**-position twin of `sequences/nextval_literal_quoted_schema_loud.sql`,
-- and the half that could not be closed while the lexer discarded quotedness: a quoted
-- `"PUBLIC"` qualifier names no schema, so the reference resolves nothing (`42P01` on
-- both). The accept side is beside it in the same statement class —
-- `PUBLIC.qpsq` and `"public".qpsq` both resolve
-- (`unquoted_public_schema_qualifier.sql`).
-- @setup
CREATE TABLE qpsq (a INT);
INSERT INTO qpsq VALUES (1);

-- @test
SELECT count(*) FROM "PUBLIC".qpsq;
-- @expect-error: a quoted schema qualifier is exact, and `PUBLIC` is not `public` (42P01)
