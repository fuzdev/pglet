-- The mirror: an unrecognized type name **before** a refused length answers `42704`,
-- which is the pair's other half and the same `LookupTypeName`-then-`typenameTypeMod`
-- sequence one column earlier.
--
-- No `@expect-message`: pglet's unrecognized-type sentence is its own
-- (`type "nosuchtype" is not supported` against 18.6's `does not exist`), a standing
-- divergence of that verdict and not of this ordering.
-- @expect-error: the first column's bad type, not the second column's refused length
-- @test
CREATE TABLE rtw2 (a nosuchtype, b CHAR(0));
