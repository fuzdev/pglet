-- The **qualified** leaf inside a computed key, and the row-count-independent half: a
-- set-op's trailing keys reach no range table on 18.6, so a qualifier anywhere in the key
-- is `42P01` there whatever the arms hold. Both arms carry a row here and it answered both
-- of them: the eval-time resolver — what makes the *bare* spelling loud once a row exists,
-- and the reason that fixture's arms are empty — never judged this one, so there is no row
-- count at which it was not silent.
--
-- `select/set_op_order_by_qualified_key_naming_an_arm_relation_loud.sql` is the whole-key
-- control that this is about the namespace and not about the name.
-- @setup
CREATE TABLE sopca (a INT);
CREATE TABLE sopcb (a INT);
INSERT INTO sopca VALUES (1);
INSERT INTO sopcb VALUES (2);

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
SELECT a FROM sopca UNION SELECT a FROM sopcb ORDER BY bogusrel.a + 1;
