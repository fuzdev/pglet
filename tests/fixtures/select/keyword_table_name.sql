-- @setup
-- A non-reserved keyword (`update`) used as a table NAME, plus a keyword column
-- name (`set`). Postgres admits these as `ColId` in *every* position — DDL,
-- INSERT, and the `FROM` clause — so pglet must accept `SELECT … FROM update`
-- too, not just `CREATE TABLE update` / `INSERT INTO update`.
CREATE TABLE update (id INT, set INT);
INSERT INTO update VALUES (1, 10), (2, 20);

-- @test
SELECT id, set FROM update WHERE set > 5 ORDER BY id DESC;
