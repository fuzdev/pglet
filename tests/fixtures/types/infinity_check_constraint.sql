-- A `CHECK` over a date judges an infinity by its order: `infinity` passes `d > '2000-01-01'`.
-- @setup
CREATE TABLE infck (d DATE CHECK (d > date '2000-01-01'));
INSERT INTO infck VALUES ('infinity');
-- @test
SELECT d FROM infck;
