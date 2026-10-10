-- `text = uuid` has no operator (`42883`). This is the shape that bit hardest:
-- the spine keys nearly every lookup on a uuid column, so a text-typed operand
-- silently returned no rows rather than reporting the mistyping.
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('11111111-1111-1111-1111-111111111111');
-- @test
-- @expect-error: no text = uuid operator
SELECT * FROM t WHERE name = '11111111-1111-1111-1111-111111111111'::uuid;
