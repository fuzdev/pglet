-- Error order, half two: a CHECK is attached before any foreign key is added, so its
-- name is already taken when the FK is looked at — and the FK's *own* name check runs
-- before its parent relation is resolved. The result is 42710, not the 42P01 the
-- missing parent would give, even though the FK is written first.
-- @expect-error: constraint dup for relation ctdk already exists
-- @test
CREATE TABLE ctdk (a INT,
  CONSTRAINT dup FOREIGN KEY (a) REFERENCES ctdk_missing(id),
  CONSTRAINT dup CHECK (a > 0));
