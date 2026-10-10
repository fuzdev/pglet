-- A row a CHECK over `mod` refuses: 23514.
-- @setup
CREATE TABLE cv (a INT CHECK (mod(a, 2) = 0));
-- @test
-- @expect-error: violates check constraint
INSERT INTO cv VALUES (3);
