-- A row an action rewrites is checked at its place in the queue, which can come before a
-- repoint queued later. The first row's move rewrites the second row's `par` first — its
-- check of every key queued then — and moves `p`'s key to `10` second, so `p`'s
-- `ON UPDATE CASCADE` onto `xp` is queued behind that check, which finds `xp = 1` gone.
-- @setup
CREATE TABLE t (id int PRIMARY KEY, xp int, par int REFERENCES t ON UPDATE CASCADE);
CREATE TABLE p (k int PRIMARY KEY REFERENCES t ON UPDATE CASCADE);
ALTER TABLE t ADD FOREIGN KEY (xp) REFERENCES p ON UPDATE CASCADE;
INSERT INTO t VALUES (1, NULL, NULL), (2, NULL, 1);
INSERT INTO p VALUES (1);
-- @test
-- @expect-error: the rewritten row's check runs before the later repoint
-- @expect-message
-- @profile-lock
UPDATE t SET id = CASE id WHEN 1 THEN 10 ELSE id END, xp = CASE id WHEN 2 THEN 1 ELSE xp END;
