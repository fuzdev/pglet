-- @no-txn
-- Its own statements, not the harness's wrapper, write the rows: 18.6 checks the key of a
-- row its own transaction wrote even where the `UPDATE` leaves the key unchanged, and here
-- that check would run before the cascade reaches the row, so inside one transaction it
-- refuses.
--
-- A row whose own key the statement leaves unchanged queues no check of it. The first row
-- references key `1`, which the second row moves; the second row's `ON UPDATE CASCADE`
-- repoints the first, and no check of the first row's unchanged key runs ahead of it.
-- @setup
DROP TABLE IF EXISTS uk_unchanged;
CREATE TABLE uk_unchanged (id int PRIMARY KEY, par int, v int);
INSERT INTO uk_unchanged VALUES (2, 1, 0), (1, NULL, 0);
ALTER TABLE uk_unchanged ADD FOREIGN KEY (par) REFERENCES uk_unchanged ON UPDATE CASCADE;
-- @test
-- @profile-lock
UPDATE uk_unchanged SET v = 1, id = CASE id WHEN 1 THEN 10 ELSE id END;
SELECT id, par, v FROM uk_unchanged ORDER BY id;
