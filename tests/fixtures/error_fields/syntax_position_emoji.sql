-- The same rule against a codepoint outside the BMP: one per emoji, not two UTF-16
-- units and not four bytes.
-- @setup
CREATE TABLE t_sp3 (a INT);

-- @test
-- @expect-error: a trailing comma in the select list
SELECT '👍👍👍' AS a, FROM t_sp3;
