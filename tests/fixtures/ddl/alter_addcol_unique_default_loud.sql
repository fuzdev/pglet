-- A constant DEFAULT under an inline UNIQUE collides across >1 existing row (23505),
-- matching Postgres's "could not create unique index".
-- @expect-error: constant default collides on the UNIQUE
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE alter_addcol_uqd (id INT);
INSERT INTO alter_addcol_uqd VALUES (1), (2);

-- @test
ALTER TABLE alter_addcol_uqd ADD COLUMN email TEXT UNIQUE DEFAULT 'x';
