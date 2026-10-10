-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup).
--
-- Inside a block every statement reads the block's start as the transaction clock —
-- across a savepoint and a `ROLLBACK TO` — while the statement clock moves with each
-- statement and is later than the block's start. Outside a block a statement is its own
-- transaction: the two clocks agree, and each statement's is later than the one before.
-- A second block has a start of its own.
-- @setup
DROP TABLE IF EXISTS cbb;
CREATE TABLE cbb (id INT, grp INT, tx TIMESTAMPTZ, stmt TIMESTAMPTZ, dflt TIMESTAMPTZ DEFAULT now());
BEGIN;
INSERT INTO cbb (id, grp, tx, stmt) VALUES (1, 1, now(), statement_timestamp());
INSERT INTO cbb (id, grp, tx, stmt) VALUES (2, 1, transaction_timestamp(), statement_timestamp());
SAVEPOINT s;
INSERT INTO cbb (id, grp, tx, stmt) VALUES (99, 1, now(), statement_timestamp());
ROLLBACK TO s;
INSERT INTO cbb (id, grp, tx, stmt) VALUES (3, 1, CURRENT_TIMESTAMP, statement_timestamp());
RELEASE s;
INSERT INTO cbb (id, grp, tx, stmt) VALUES (4, 1, now(), statement_timestamp());
COMMIT;
INSERT INTO cbb (id, grp, tx, stmt) VALUES (5, 2, now(), statement_timestamp());
INSERT INTO cbb (id, grp, tx, stmt) VALUES (6, 2, now(), statement_timestamp());
BEGIN;
INSERT INTO cbb (id, grp, tx, stmt) VALUES (7, 3, now(), statement_timestamp());
INSERT INTO cbb (id, grp, tx, stmt) VALUES (8, 3, now(), statement_timestamp());
COMMIT;
-- @test
SELECT g.grp, g.written_rows, g.transaction_values, g.statement_values, g.default_differs,
       g.clocks_agree, g.statement_later,
       g.last_tx < n.first_tx AS before_the_next_group,
       g.last_stmt < now() AS before_this_statement
FROM (SELECT grp,
             count(*) AS written_rows,
             count(DISTINCT tx) AS transaction_values,
             count(DISTINCT stmt) AS statement_values,
             sum(CASE WHEN tx = dflt THEN 0 ELSE 1 END) AS default_differs,
             sum(CASE WHEN tx = stmt THEN 1 ELSE 0 END) AS clocks_agree,
             sum(CASE WHEN tx < stmt THEN 1 ELSE 0 END) AS statement_later,
             max(tx) AS last_tx,
             max(stmt) AS last_stmt
      FROM cbb GROUP BY grp) g
LEFT JOIN (SELECT grp, min(tx) AS first_tx FROM cbb GROUP BY grp) n ON n.grp = g.grp + 1
ORDER BY g.grp;
