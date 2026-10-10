-- An aggregate as a set-op trailing sort key is `0A000` on both engines, by different
-- causes: 18.6's invalid-UNION-ORDER-BY rule, pglet's own aggregate-eval-context
-- refusal. The agreement is what makes this fixture possible where the ordinary
-- computed-key leniency (`-a`, `a+1` — pglet sorts, 18.6 `0A000`) cannot be pinned.
-- @setup
CREATE TABLE soa (a INT);
CREATE TABLE sob (a INT);
INSERT INTO soa VALUES (1);
INSERT INTO sob VALUES (2);
-- @expect-error: aggregate in a set-op ORDER BY
-- @test
SELECT a FROM soa UNION SELECT a FROM sob ORDER BY count(*);
