-- An unquoted part of a name literal is **downcased**, so `nextval('MySeq')` resolves
-- the sequence `CREATE SEQUENCE myseq` registered and answers 1 — the same fold
-- `Token::name` applies in an identifier position, which is what makes the two name
-- positions name one relation: `CREATE SEQUENCE MySeq` also registers `myseq`, and
-- this literal finds it either way.
-- The quoted half is the discriminator and lives beside this: a quoted relation part
-- stays exact (`nextval_literal_quoted_relation_loud.sql`), and a quoted schema
-- qualifier names no schema (`nextval_literal_quoted_schema_loud.sql`). The
-- accept-side spellings of the qualifier are in
-- `nextval_literal_schema_qualifier_case.sql`.
-- @setup
CREATE SEQUENCE myseq;
-- @test
SELECT nextval('MySeq');
