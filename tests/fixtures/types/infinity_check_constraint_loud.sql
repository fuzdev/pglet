-- `-infinity` fails the same `CHECK`.
-- @setup
CREATE TABLE infckl (d DATE CONSTRAINT infckl_d CHECK (d > date '2000-01-01'));
-- @expect-error: violates check constraint
-- @expect-message
-- @test
INSERT INTO infckl VALUES ('-infinity');
