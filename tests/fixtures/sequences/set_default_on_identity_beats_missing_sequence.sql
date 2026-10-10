-- The identity refusal outranks the default expression's own resolution: on 18.6 the
-- named sequence is coerced to `regclass` during parse analysis, yet
-- `SET DEFAULT nextval('<no such sequence>')` on an identity column is still 42601,
-- not 42P01.
--
-- It therefore also outranks pglet's DDL-time `ensure_sequence_relation`, which
-- registers the named sequence and stands in for a *runtime* 42809 — that gate runs
-- last by rule, so anything Postgres raises at DDL time has to be ahead of it.
-- @setup
CREATE TABLE sdims_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER TABLE sdims_t ALTER COLUMN id SET DEFAULT nextval('nosuch_sdims_seq');
-- @expect-error: the identity refusal outranks the default's sequence (42601)
