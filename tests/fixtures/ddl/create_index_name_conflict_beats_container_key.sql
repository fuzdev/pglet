-- pglet's refusal to build a non-unique B-tree over a single bare container column
-- stands in for **no** Postgres error — `jsonb` has a default B-tree opclass
-- (`jsonb_ops`), so Postgres builds this index happily and the only thing wrong with the
-- statement is the taken name. A gate standing in for nothing Postgres raises must
-- therefore not outrank the name conflict: this is `42P07` on both engines, where the
-- reordered method check made pglet answer `0A000`.
-- @setup
CREATE TABLE cincbk_t (a INT, j JSONB);
CREATE INDEX cincbk_ix ON cincbk_t (a);
-- @test
CREATE INDEX cincbk_ix ON cincbk_t (j);
-- @expect-error: the taken name outranks pglet's own container-key restriction (42P07)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
