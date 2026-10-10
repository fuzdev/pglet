-- The qualified spelling of the same hole, and the worse half: it is **row-count
-- independent**, so no seeded row ever made it loud. A set-op's trailing keys reach no
-- range table on 18.6 — only the set-op's own output names may appear there — so any
-- qualifier is `42P01`, and pglet answered rows.
--
-- `select/set_op_order_by_qualified_key_naming_an_arm_relation_loud.sql` is the control
-- that this is about the namespace and not about the name: a qualifier naming a *real*
-- arm relation is `42P01` on both engines too.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);
INSERT INTO sopa VALUES (1);
INSERT INTO sopb VALUES (2);

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY bogusrel.a;
