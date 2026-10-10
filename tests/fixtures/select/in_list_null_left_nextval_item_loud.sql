-- Under a NULL left operand an `IN` list item is evaluated, and a `nextval` item, which
-- pglet evaluates only at a statement's top level, is refused rather than skipped:
-- skipped, it would answer NULL without drawing the value 18.6 draws.
-- @setup
CREATE SEQUENCE s;
-- @expect-error: nextval() is only supported as a top-level SELECT expression
-- @test
SELECT NULL::int IN (1, nextval('s')::int) AS a;
