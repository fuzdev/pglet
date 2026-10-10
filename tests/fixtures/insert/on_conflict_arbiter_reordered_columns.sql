-- **The member that says this is a set rule and not a duplicate rule.** There is no
-- repeat here at all — the inference list simply names the index's two key columns in the
-- other order, which `bms_equal` cannot tell apart. A fix scoped to "collapse duplicates"
-- leaves this file red; pglet answered `42P10`.
-- @setup
CREATE TABLE oaro (a INT, b INT, UNIQUE (a, b));
INSERT INTO oaro VALUES (1, 1);
INSERT INTO oaro VALUES (1, 1) ON CONFLICT (b, a) DO NOTHING;

-- @test
SELECT a, b FROM oaro ORDER BY a;
