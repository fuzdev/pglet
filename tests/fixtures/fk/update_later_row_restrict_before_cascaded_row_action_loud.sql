-- The actions a cascaded row queues run after every action already queued. The first row's
-- `ON UPDATE CASCADE` moves `c`'s key, whose `ON UPDATE RESTRICT` child `d` refuses it —
-- but the second row's own `RESTRICT` child `r` was queued first, and `r_k_fkey` refuses.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE c (k int UNIQUE REFERENCES p ON UPDATE CASCADE);
CREATE TABLE r (k int REFERENCES p ON UPDATE RESTRICT);
CREATE TABLE d (k int REFERENCES c (k) ON UPDATE RESTRICT);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (1);
INSERT INTO d VALUES (1);
INSERT INTO r VALUES (2);
-- @test
-- @expect-error: the second row's RESTRICT before the cascaded row's
-- @expect-message
-- @profile-lock
UPDATE p SET k = k + 10;
