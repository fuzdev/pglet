-- Both halves of the rule at once, over a composite key: a repeat collapses and the
-- remaining members need not be in key order.
-- @setup
CREATE TABLE oarc (a INT, b INT, UNIQUE (a, b));
INSERT INTO oarc VALUES (1, 1);
INSERT INTO oarc VALUES (1, 1) ON CONFLICT (a, a, b) DO NOTHING;

-- @test
SELECT a, b FROM oarc ORDER BY a;
