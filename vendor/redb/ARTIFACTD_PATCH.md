# Maintained redb cache patch

This directory vendors redb 4.3.0 with a narrow page-cache queue repair copied from upstream commit `f95d4609137481e6e40f04d9a103aa9541993267`. The transactional engine and on-disk format are unchanged. MIT and Apache-2.0 notices are retained.

The repair adds a generation to each queue token and cached entry. Queue maintenance and eviction discard stale generations after a page offset is reused, while preserving value replacement and second-chance behavior.

Replace this backport with a reviewed upstream release containing the repair when available. Review upstream changes before updating this fork.
