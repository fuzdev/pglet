-- An outer frame that owns the qualifier and lacks the column is a **column** error, not
-- a relation one: the qualifier resolved, so 18.6 says `42703 column h.nosuch does not
-- exist`. pglet answered `42P01 "h"` — loud, and the wrong half of the binding family —
-- because the chain could only say "some frame claimed this" by producing a slot, and a
-- missing column produced no slot at all.
--
-- The accept polarity is the ordinary `olc_u.yv = h.v`, which the corpus exercises
-- widely; the neighbouring reject is `zzz.v`, still `42P01` on both, and the pair is what
-- says the split is by *whether a frame owns the qualifier* rather than by spelling.
-- @setup
CREATE TABLE olc_hw (id INT, v TEXT);
CREATE TABLE olc_u (yv TEXT);
INSERT INTO olc_hw VALUES (1, 'hit');
INSERT INTO olc_u VALUES ('hit');

-- @expect-error: column h.nosuch does not exist (42703)
-- @test
SELECT id FROM olc_hw h WHERE EXISTS (SELECT 1 FROM olc_u WHERE olc_u.yv = h.nosuch);
