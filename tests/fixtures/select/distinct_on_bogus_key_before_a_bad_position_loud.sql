-- The interleave one clause over: a `DISTINCT ON` key list is resolved per key too, so the
-- bare name first is `42703` and `select/distinct_on_bad_position_before_a_bogus_key_loud
-- .sql` — the same two faults swapped — is `42P10`. This half was `42P10` here.
--
-- The **no-FROM** spelling of the same pair is
-- `select/distinct_on_no_from_bogus_key_before_a_bad_position_loud.sql`; the two sites are
-- different functions with the same rule, which is why both pairs exist.
-- @setup
CREATE TABLE sk_don (a INT, b INT);
INSERT INTO sk_don VALUES (1, 10);

-- @expect-error: column "bogusdon" does not exist (42703)
-- @test
SELECT DISTINCT ON (bogusdon, 9) a FROM sk_don;
