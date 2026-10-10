-- A qualifier a level owns names one relation, and binds only a column of that relation.
-- Over a join, the aggregate / sort / window positions read the join's `"relation.column"`
-- names; where the owning relation lacks the column, a suffix match took a **sibling's**
-- same-named column (`max(o.k)` answered `cvq_w.k`, `2`) where 18.6 refuses `42703`. A
-- catalog view reaches it because the plan-time gate cannot close over a relation pglet
-- models a subset of the columns of; an ordinary table is refused there first.
--
-- This one: an `information_schema` view as the owner.
-- @setup
CREATE TABLE cvq_w (k INT, n INT);
CREATE TABLE cvq_t3 (y INT, relname TEXT, n INT, k INT);
INSERT INTO cvq_w VALUES (1, 100), (2, 200);
INSERT INTO cvq_t3 VALUES (1, 'zz', 10, 50), (2, 'zz', 20, 60);

-- @expect-error: column "o.k" does not exist (42703)
-- @test
SELECT max(o.k) FROM information_schema.tables o JOIN cvq_w ON true;
