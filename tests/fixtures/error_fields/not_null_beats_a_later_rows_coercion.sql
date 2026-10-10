-- Row-scoped is not the same as statement-scoped. 18.6 constrains each new tuple inside
-- `ExecUpdate` before it evaluates the next, so row 1's null outranks row 2's enum
-- input-function failure. A gate that waits for the whole collect pass answers `22P02`
-- instead — which is what a NOT NULL check hoisted out of the per-row loop did.
-- @setup
CREATE TYPE ee AS ENUM ('r', 'g');
CREATE TABLE y1 (a INT NOT NULL, e ee, k INT);
INSERT INTO y1 VALUES (1, 'r', 1), (2, 'g', 2);

-- @test
-- @expect-error: row 1's null, not row 2's bad enum label
-- @expect-message
UPDATE y1 SET a = CASE WHEN k = 1 THEN NULL ELSE 8 END,
              e = CASE WHEN k = 2 THEN 'purple' ELSE 'r' END::ee;
