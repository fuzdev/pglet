-- A check of a version an action has since rewritten is skipped, though the row as it stands
-- would fail it. The third row newly references `p` key `1`, and its check is queued with
-- it. Before that check's turn, the first row's move cascades `p`'s key to `10` — whose own
-- `ON UPDATE CASCADE` repoints `xp` only later in the queue — and the second row's move
-- rewrites the third row's `par`, superseding the version the check was queued for. The
-- skipped check would have found `xp = 1` gone; the rewrite's own check runs after the
-- repoint.
-- @setup
CREATE TABLE t (id int PRIMARY KEY, xp int, par int REFERENCES t ON UPDATE CASCADE);
CREATE TABLE p (k int PRIMARY KEY REFERENCES t ON UPDATE CASCADE);
ALTER TABLE t ADD FOREIGN KEY (xp) REFERENCES p ON UPDATE CASCADE;
INSERT INTO t VALUES (1, NULL, NULL), (2, NULL, NULL), (3, NULL, 2);
INSERT INTO p VALUES (1);
-- @test
-- @profile-lock
UPDATE t SET id = CASE id WHEN 1 THEN 10 WHEN 2 THEN 20 ELSE id END, xp = CASE id WHEN 3 THEN 1 ELSE xp END;
SELECT id, xp, par FROM t ORDER BY id;
