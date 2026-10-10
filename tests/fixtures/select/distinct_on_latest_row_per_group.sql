-- @setup
CREATE TABLE la (aid INT, label TEXT);
CREATE TABLE lb (a_id INT, created_at INT, note TEXT);
INSERT INTO la VALUES (1, 'one'), (2, 'two');
INSERT INTO lb VALUES (1, 5, 'early-1'), (1, 9, 'late-1'), (2, 1, 'early-2'), (2, 7, 'late-2');

-- The latest-row-per-group idiom, the single most common use of DISTINCT ON. The
-- ON key `a.aid` *is* projected, so canonicalization strips its qualifier to the
-- bare output name — while `b.created_at` is dropped, so the Sort has to sit below
-- the projection to see it. Mixing an output-answered key with a dropped one in the
-- one ORDER BY is exactly the shape at issue. `b.note` is projected so the row each
-- group keeps is visible: DESC picks the late note, and a sort that lost
-- `created_at` (or read it as ASC) would pick the early one.
-- @test
SELECT DISTINCT ON (a.aid) a.aid, b.note FROM la a JOIN lb b ON b.a_id = a.aid ORDER BY a.aid, b.created_at DESC;
