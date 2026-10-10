-- DROP TABLE ... CASCADE drops the parent and **only the child's FK constraint**:
-- the child table and its rows survive, and the insert the FK used to reject is
-- then accepted (measured against Postgres 18.6).
-- @setup
CREATE TABLE dtc_par (id INT PRIMARY KEY);
CREATE TABLE dtc_ch (cid INT PRIMARY KEY, pid INT REFERENCES dtc_par(id));
INSERT INTO dtc_par VALUES (1);
INSERT INTO dtc_ch VALUES (10, 1);

-- @profile-lock
-- @test
DROP TABLE dtc_par CASCADE;
INSERT INTO dtc_ch VALUES (11, 999);
SELECT cid, pid FROM dtc_ch ORDER BY cid;
