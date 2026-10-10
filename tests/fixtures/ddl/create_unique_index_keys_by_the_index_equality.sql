-- The build gate has to decide "would this index already hold two entries it calls
-- equal?", and *which* equality answers that is the whole question. A structural
-- (postcard) comparison over the `Value` tuple is **not** the index's: the key encoding
-- is order-preserving, so it normalises what ordering cannot distinguish, and a
-- `numeric` `1.5` and `1.50` are one key there and two values here.
--
-- With the structural basis this index was built, after which pglet's **own** write path
-- refused the row already sitting in the table — the engine disagreeing with itself. The
-- three validation scans (this one, `ADD CONSTRAINT UNIQUE`/`ADD PRIMARY KEY`, and the
-- `ADD COLUMN` backfill) now key through one function, `index_key_identity`.
--
-- The `DETAIL` names the **first** row in scan order, not the row that collided —
-- measured both ways round on 18.6 (`1.50` then `1.5` reads `Key (n)=(1.50)`), which is
-- the distinction a colliding-row report gets wrong exactly when the two spellings
-- differ, i.e. exactly on this fixture's shape.
-- @setup
CREATE TABLE cuke_t (n NUMERIC);
INSERT INTO cuke_t VALUES (1.50), (1.5);
-- @test
CREATE UNIQUE INDEX cuke_ix ON cuke_t (n);
-- @expect-error: the index's own equality calls 1.50 and 1.5 one key (23505)
-- @expect-message
-- @profile-lock
