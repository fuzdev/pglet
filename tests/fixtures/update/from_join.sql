-- `UPDATE t alias SET … FROM other a WHERE …` — the target is inner-joined to the
-- source; only target rows with a matching source row are updated. Run the
-- mutation in setup, then read back the deterministic resulting state.
-- @setup
CREATE TABLE offer (id INT, to_account INT, superseded BOOL);
INSERT INTO offer VALUES (1, 9001, false), (2, 9002, false), (3, 9001, false);
CREATE TABLE actor (id INT, account_id INT);
INSERT INTO actor VALUES (100, 9001), (101, 9002);
UPDATE offer o
SET superseded = true
FROM actor a
WHERE a.id = 100 AND o.to_account = a.account_id;
-- @test
SELECT id, to_account, superseded FROM offer ORDER BY id;
