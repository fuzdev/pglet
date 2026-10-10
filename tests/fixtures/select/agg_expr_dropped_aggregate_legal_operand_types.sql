-- The accept side of the dropped aggregate's operand type check: an aggregate that has a
-- form for its operand's type is dropped with its branch and the statement answers —
-- `max(text)`, `count` over any type, `min` over a cast to text, `max(int)`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @test
SELECT true OR max(t) > 'a' AS text_max, true OR count(j) > 0 AS count_any,
  false AND min(a::text) > 'a' AS cast_min, CASE WHEN false THEN max(t) END AS case_arm,
  coalesce(1, max(a)) AS coalesce_rest, CASE 1 WHEN 2 THEN min(a) ELSE 5 END AS simple_case
FROM o;
