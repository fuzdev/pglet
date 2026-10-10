-- A CASCADE repoint that would violate the child's CHECK is rejected (23514): the
-- repoint re-validates the child's CHECK before writing.
-- @profile-lock
-- @setup
CREATE TABLE oucv_p (id INT PRIMARY KEY);
CREATE TABLE oucv_c (pid INT REFERENCES oucv_p(id) ON UPDATE CASCADE CHECK (pid < 100));
INSERT INTO oucv_p VALUES (5);
INSERT INTO oucv_c VALUES (5);

-- @test
-- @expect-error: repointing pid to 200 violates CHECK (pid < 100) (23514)
UPDATE oucv_p SET id = 200 WHERE id = 5;
