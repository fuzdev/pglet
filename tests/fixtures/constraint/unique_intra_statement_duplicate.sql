-- Two rows in a single VALUES list sharing a key violate the unique constraint
-- (the duplicate is caught within the same statement).
-- @profile-lock
-- @setup
CREATE TABLE uq_intra (id INT PRIMARY KEY);

-- @test
-- @expect-error: intra-statement duplicate key violates unique constraint
INSERT INTO uq_intra VALUES (1), (1);
