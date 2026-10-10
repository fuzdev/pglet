-- The gate is keyed on the **tree being a set op**, not on which parser arm called the
-- lowering, and this is the spelling that separates the two. A chain whose first operand
-- is parenthesized reaches `plan_set_op_order_limit` through a different arm of
-- `parse_select_inner`, and 18.6 treats its trailing `ORDER BY` exactly as the unbracketed
-- chain's — `42703` for a name no output column carries.
--
-- `select/set_op_order_by_paren_leading_plain_select_may_name_an_inner_column.sql` is the
-- accept polarity that the same arm serves, and the reason a per-arm gate would be wrong.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
(SELECT a FROM sopa) UNION SELECT a FROM sopb ORDER BY bogus;
