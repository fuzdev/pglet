-- **The interleave**, first half. A key's binding and its positional range check are two
-- verdicts on one clause, and Postgres reaches them one key at a time
-- (`findTargetlistEntrySQL92` is called per `SortBy`), so the positionally **first** wrong
-- key decides: here the bare name, `42703`. Its twin
-- (`select/order_by_bad_position_before_a_bogus_key_loud.sql`) swaps the two and is
-- `42P10`, and no arrangement of one whole-list binding pass against one whole-list range
-- pass yields both.
--
-- This half was `42P10` here until the keys gained a claimant set to bind against: with no
-- binding verdict to reach at all, the range check answered whatever its position.
-- @setup
CREATE TABLE sk_pos (a INT, b INT);
INSERT INTO sk_pos VALUES (1, 10);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT a FROM sk_pos ORDER BY bogus, 9;
