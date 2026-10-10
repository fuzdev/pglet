-- Under a NULL left operand a `currval` item of an `IN` list is refused, as it is under
-- any operand. 18.6 answers NULL here, after the session's first `nextval`; skipped, the
-- item would also answer NULL before it, where 18.6 raises `55000`.
-- @setup
CREATE SEQUENCE s;
SELECT nextval('s');
-- @expect-error: currval() is only supported as a top-level SELECT expression
-- @test
SELECT NULL::int IN (1, currval('s')::int) AS a;
