-- The same comparison gate on `ALTER TABLE … ADD CONSTRAINT … CHECK` (`42883`).
-- @setup
CREATE TABLE alt_ck_cmp (name TEXT);
-- @test
-- @expect-error: no text = integer operator
ALTER TABLE alt_ck_cmp ADD CONSTRAINT alt_ck_cmp_c CHECK (name = 5);
