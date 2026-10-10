-- The error-order arrangement only a **computed** key can discriminate. The whole-key
-- twin (`select/set_op_order_by_bogus_key_before_a_bad_position_loud.sql`) already passed
-- while a computed key escaped the gate entirely, so this spelling answered the *second*
-- key's `42P10` where 18.6 answers the first key's `42703`.
--
-- What it pins is that the leaf walk runs inside the same per-key closure as the
-- positional range check, rather than as a pass of its own: the reverse arrangement
-- (`ORDER BY 99, bogus + 1`) is `42P10` on both engines and was `42P10` here before this
-- gate existed too, so it discriminates nothing and has no fixture.
-- @setup
CREATE TABLE sopca (a INT);
CREATE TABLE sopcb (a INT);

-- @expect-error: column "bogus" does not exist, ahead of the bad ordinal (42703)
-- @test
SELECT a FROM sopca UNION SELECT a FROM sopcb ORDER BY bogus + 1, 99;
