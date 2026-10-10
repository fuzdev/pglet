-- **The sort-key binding exemption's plainest member.** A level's own `ORDER BY` keys
-- used to be resolved *before* the projection existed, against a scope whose binding gate
-- declined to judge every bare name — the only shape it could take, since the claimant
-- set the rule calls for is the target list. A key naming nothing therefore survived plan
-- time, and over a relation with **no rows** the eval-time resolver never ran either, so
-- nothing judged it: this answered `(0 rows)` where 18.6 is `42703`.
--
-- **Emptiness is load-bearing for the read polarity and only for it.** The same statement
-- over a seeded relation was already `42703` here, from the eval-time resolver — which is
-- what identifies the missing plan-time judgement rather than a missing rule. The write
-- polarity is row-count-independent instead, because an unreferenced CTE's body never
-- executes at any row count
-- (`select/order_by_key_binds_to_nothing_in_a_cte_delete_loud.sql`).
--
-- The keys are resolved after the projection now, against the target list as an explicit
-- claimant set, so `findTargetlistEntrySQL92` falls through to the range table exactly as
-- Postgres's does.
-- @setup
CREATE TABLE sk_empty (a INT, b INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT a FROM sk_empty ORDER BY bogus;
