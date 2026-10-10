-- @no-txn
-- Its own statements, not the harness's wrapper, write the rows: 18.6 checks every key of a
-- row its own transaction wrote, so inside one transaction the rewritten row's unchanged
-- `w` would be checked before `nw_w`'s cascade repoints it, and the statement refuses.
--
-- A row neither the statement nor an earlier action wrote is checked, when an action
-- rewrites it, only on the key the action changed. The `UPDATE` moves `nw_s.id` `1` to `2`,
-- which cascades `nw_t` `1` to `2`; `nw_t`'s actions repoint row `100`'s `t` and move
-- `nw_w` `1` to `2`, whose cascade then repoints row `100`'s `w`. A check of row `100`'s
-- unchanged `w` between the two would find `nw_w` `1` gone and refuse.
-- @setup
DROP TABLE IF EXISTS nw_s, nw_t, nw_w CASCADE;
CREATE TABLE nw_t (k int PRIMARY KEY);
CREATE TABLE nw_s (id int PRIMARY KEY, t int REFERENCES nw_t ON UPDATE CASCADE, w int);
CREATE TABLE nw_w (k int PRIMARY KEY REFERENCES nw_t ON UPDATE CASCADE);
INSERT INTO nw_s VALUES (1, NULL, NULL);
INSERT INTO nw_t VALUES (1);
INSERT INTO nw_w VALUES (1);
INSERT INTO nw_s VALUES (100, 1, 1);
ALTER TABLE nw_s ADD FOREIGN KEY (w) REFERENCES nw_w ON UPDATE CASCADE;
ALTER TABLE nw_t ADD FOREIGN KEY (k) REFERENCES nw_s ON UPDATE CASCADE;
-- @test
-- @profile-lock
UPDATE nw_s SET id = 2 WHERE id = 1;
SELECT s.id, s.t, s.w, (SELECT k FROM nw_t) AS tk, (SELECT k FROM nw_w) AS wk FROM nw_s s ORDER BY s.id;
