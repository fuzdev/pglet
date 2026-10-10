-- A correlated subquery stringifying an **outer** column (`'v=' || o.a`,
-- `o.a::text`) must keep the `+00` offset. The outer row reaches the subquery as an
-- `OuterFrame`, so the frame — not just the inner scope's resolver — has to carry
-- the outer scope's declared types; without them the correlated reference rendered
-- offset-less while the identical inner reference did not. The last column nests two
-- levels deep, so the type must survive the whole frame chain.
-- @setup
CREATE TABLE ctzs (id INT, a TIMESTAMPTZ);
INSERT INTO ctzs VALUES (1, '2026-07-18 12:30:45+00'), (2, '2026-07-19 01:02:03+00');
-- @test
SELECT
  id,
  (SELECT 'v=' || i.a FROM ctzs i WHERE i.id = o.id) AS inner_concat,
  (SELECT 'v=' || o.a FROM ctzs i WHERE i.id = o.id) AS outer_concat,
  (SELECT o.a::text FROM ctzs i WHERE i.id = o.id) AS outer_cast,
  (SELECT (SELECT 'w=' || o.a FROM ctzs j WHERE j.id = o.id) FROM ctzs i WHERE i.id = o.id) AS outer_nested
FROM ctzs o
ORDER BY id;
