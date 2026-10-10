-- SET NULL at a middle level propagates: a <- b (SET NULL) <- cc (CASCADE). Nulling
-- b's key cascades that change (old value -> NULL) onto the grandchild.
-- @profile-lock
-- @setup
CREATE TABLE osnm_a (id INT PRIMARY KEY);
CREATE TABLE osnm_b (bid INT PRIMARY KEY, aid INT UNIQUE REFERENCES osnm_a(id) ON UPDATE SET NULL);
CREATE TABLE osnm_cc (cid INT PRIMARY KEY, bref INT REFERENCES osnm_b(aid) ON UPDATE CASCADE);
INSERT INTO osnm_a VALUES (1);
INSERT INTO osnm_b VALUES (10, 1);
INSERT INTO osnm_cc VALUES (100, 1);
UPDATE osnm_a SET id = 2;

-- @test
SELECT
  (SELECT aid FROM osnm_b WHERE bid = 10) AS b_aid,
  (SELECT bref FROM osnm_cc WHERE cid = 100) AS cc_bref;
