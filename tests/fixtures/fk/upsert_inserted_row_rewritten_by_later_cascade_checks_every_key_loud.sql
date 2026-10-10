-- A row an `ON CONFLICT DO UPDATE` inserts is the statement's own write, so an action that
-- later rewrites it checks every one of its keys, not only the one the action changed.
-- The `DO UPDATE` moves `ni_s.id` `1` to `2`, which cascades `ni_t` `1` to `2`; `ni_t`'s
-- actions then repoint the inserted row's `t` (queueing a check of its `w` too) and move
-- `ni_w` `1` to `2`, whose own cascade into `ni_s.w` comes later in the queue. The inserted
-- row's `w` check fires between the two and finds `ni_w` `1` gone. Were the inserted row
-- not counted as the statement's own, only its changed `t` would be checked and the later
-- cascade would repoint `w` with no refusal.
-- @setup
CREATE TABLE ni_t (k int PRIMARY KEY);
CREATE TABLE ni_s (id int PRIMARY KEY, t int REFERENCES ni_t ON UPDATE CASCADE, w int);
CREATE TABLE ni_w (k int PRIMARY KEY REFERENCES ni_t ON UPDATE CASCADE);
INSERT INTO ni_s VALUES (1, NULL, NULL);
INSERT INTO ni_t VALUES (1);
INSERT INTO ni_w VALUES (1);
ALTER TABLE ni_s ADD FOREIGN KEY (w) REFERENCES ni_w ON UPDATE CASCADE;
ALTER TABLE ni_t ADD FOREIGN KEY (k) REFERENCES ni_s ON UPDATE CASCADE;
-- @test
-- @profile-lock
-- @expect-error: insert or update on table "ni_s" violates foreign key constraint "ni_s_w_fkey"
INSERT INTO ni_s VALUES (100, 1, 1), (1, NULL, NULL) ON CONFLICT (id) DO UPDATE SET id = 2;
