-- The sub-select of an `ANY ((SELECT …))` runs ahead of pglet's refusal of the form, so a
-- division in it raises as it does on 18.6.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
-- @expect-error: division by zero
-- @test
SELECT 1 = ANY ((SELECT 1 / (a - a) FROM r));
