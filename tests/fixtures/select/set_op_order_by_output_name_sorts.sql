-- The accept polarity of the set-op key gate: a key naming an output column still binds
-- and still sorts. The left arm names the output columns, so `a` is that name; the rows
-- come back ascending across both arms.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);
INSERT INTO sopa VALUES (3), (1);
INSERT INTO sopb VALUES (2);

-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY a;
