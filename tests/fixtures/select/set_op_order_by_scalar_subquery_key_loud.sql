-- A scalar subquery as a set-op trailing sort key is `0A000` on both engines, by
-- different causes: 18.6's invalid-UNION-ORDER-BY rule, pglet's
-- scalar-subquery-execution-context refusal. Pinned for the same reason as the
-- aggregate sibling: the SQLSTATEs agree where the plain computed-key leniency's do not.
-- @setup
CREATE TABLE soc (a INT);
CREATE TABLE sod (a INT);
INSERT INTO soc VALUES (1);
INSERT INTO sod VALUES (2);
-- @expect-error: scalar subquery in a set-op ORDER BY
-- @test
SELECT a FROM soc UNION SELECT a FROM sod ORDER BY (SELECT 1);
