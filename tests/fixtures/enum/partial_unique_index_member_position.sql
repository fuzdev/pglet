-- A **unique partial** index whose predicate orders an enum: the member order decides
-- which rows are in the index, and therefore which duplicates are refused. Both rows
-- carry a member declared *after* `sad`, so both are indexed and the second is `23505`
-- — where a label-byte comparison puts neither row in the index and the duplicate lands
-- silently.
--
-- The membership rule lives in two objects that a query never compares (the executor's
-- `PreparedIndex` and the storage layer's `IndexKeying`), which is why this is the
-- one enum shape whose failure is a **write** rather than a wrong row order.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE pt (id INT, m mood NOT NULL);
CREATE UNIQUE INDEX pt_u ON pt (id) WHERE m > 'sad';
INSERT INTO pt VALUES (1, 'ok');

-- @profile-lock
-- @test
-- @expect-error: both rows are in the partial index, so the duplicate id collides
INSERT INTO pt VALUES (1, 'happy');
