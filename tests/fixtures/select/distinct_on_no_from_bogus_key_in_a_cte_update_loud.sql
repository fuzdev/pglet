-- The `UPDATE` polarity of `select/distinct_on_no_from_bogus_key_in_a_cte_delete_loud.sql`,
-- kept as its own fixture because the two statements reach the unjudged key through
-- different DML lowerings: this one overwrote every row with `99` where 18.6 is `42703`
-- and leaves them. Emptiness is not load-bearing here either — see its twin's header.
--
-- Row survival is asserted by `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_no_from_distinct_on_key_binding_to_nothing_does_not_update`, for the reason its twin
-- states: an `@expect-error` fixture is green for a refusal raised after the writes.
-- @setup
CREATE TABLE dnf_n9 (a INT);
INSERT INTO dnf_n9 VALUES (1), (2);

-- @expect-error: column "bogusdon" does not exist (42703)
-- @test
WITH c AS (SELECT DISTINCT ON (bogusdon) unnest(ARRAY[]::int[]) AS q) UPDATE dnf_n9 SET a = 99;
