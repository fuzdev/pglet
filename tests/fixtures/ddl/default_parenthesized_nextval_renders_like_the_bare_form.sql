-- A parenthesized recognized `DEFAULT` reaches `information_schema` as the bare
-- form's rendering, because 18.6 strips the redundant parens in parse analysis and
-- pglet reduces the same tree onto the same closed catalog arm. Measured: both
-- columns read `nextval('dpr_seq'::regclass)` on both engines.
--
-- The `::text` casts are the corpus idiom for this query (see
-- `information_schema/column_defaults.sql`): `information_schema.columns` declares
-- `name` (OID 19) and `character_data` (a domain over `varchar`) where pglet models
-- neither and reports `text`, so projecting the raw columns would need a
-- `@no-declared-types:` waiver over a gap that is not this fixture's subject. Casting
-- makes both engines declare `text` and keeps the declared-type comparison live.
-- @setup
CREATE SEQUENCE dpr_seq;
CREATE TABLE dpr (a INT DEFAULT (nextval('dpr_seq')), b INT DEFAULT nextval('dpr_seq'), c INT DEFAULT (5), d INT DEFAULT 5);
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default FROM information_schema.columns WHERE table_name = 'dpr' ORDER BY ordinal_position;
