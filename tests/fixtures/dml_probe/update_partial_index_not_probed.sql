-- A partial B-tree index serves no unconditional probe, so the UPDATE scans and finds the
-- rows the index does not hold as well as the one it does.
-- @profile-lock
-- @setup
CREATE TABLE pt (k INT, v TEXT);
CREATE INDEX pt_k ON pt (k) WHERE k > 10;
INSERT INTO pt VALUES (5, 'a'), (20, 'b');
-- @test
UPDATE pt SET v = v || '!' WHERE k = 20 OR k = 5;
UPDATE pt SET v = v || '?' WHERE k = 5 RETURNING k, v;
