-- A row's own key is checked before the next row's actions run, so the first row's missing
-- parent — `t_qk_fkey`, `insert or update` — outranks the second row's `NO ACTION`
-- refusal, the same SQLSTATE on another constraint.
-- @setup
CREATE TABLE q (k int PRIMARY KEY);
INSERT INTO q VALUES (1);
CREATE TABLE t (id int PRIMARY KEY, qk int REFERENCES q);
CREATE TABLE na (tid int REFERENCES t);
INSERT INTO t VALUES (1, 1), (2, 1);
INSERT INTO na VALUES (2);
-- @test
-- @expect-error: the first row's key before the second row's NO ACTION
-- @expect-message
-- @profile-lock
UPDATE t SET qk = CASE id WHEN 1 THEN 99 ELSE qk END, id = CASE id WHEN 2 THEN 200 ELSE id END;
