-- `RESTART IDENTITY` covers the whole truncation closure, so a child pulled in by
-- `CASCADE` has its own sequence restarted too. (The clause order is fixed: `CASCADE`
-- after `RESTART IDENTITY`.)
-- @setup
CREATE TABLE tricc_p (id INT PRIMARY KEY);
CREATE TABLE tricc_c (cid SERIAL, pid INT REFERENCES tricc_p(id));
INSERT INTO tricc_p (id) VALUES (1);
INSERT INTO tricc_c (pid) VALUES (1), (1);
TRUNCATE tricc_p RESTART IDENTITY CASCADE;
INSERT INTO tricc_p (id) VALUES (1);
INSERT INTO tricc_c (pid) VALUES (1);
-- @test
SELECT cid, pid FROM tricc_c;
