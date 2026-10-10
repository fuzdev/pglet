-- @expect-error: cannot truncate an FK-referenced table without CASCADE
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE trunc_ep (id INT PRIMARY KEY);
CREATE TABLE trunc_ec (id INT, parent_id INT REFERENCES trunc_ep(id));
INSERT INTO trunc_ep VALUES (1);
INSERT INTO trunc_ec VALUES (10, 1);

-- @test
TRUNCATE trunc_ep;
