-- The `UPDATE` polarity of `select/order_by_key_binds_to_nothing_in_a_cte_delete_loud.sql`,
-- which overwrote every row of the target with `99`. Row survival:
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_sort_key_binding_to_nothing_does_not_update`.
-- @setup
CREATE TABLE sk_usrc (a INT);
CREATE TABLE sk_utgt (a INT);
INSERT INTO sk_usrc VALUES (7);
INSERT INTO sk_utgt VALUES (1), (2);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
WITH c AS (SELECT a FROM sk_usrc ORDER BY bogus) UPDATE sk_utgt SET a = 99;
