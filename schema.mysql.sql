-- Generated from an initialized Rizzy SQLite database.
-- Search tables, triggers, and application queries require a separate MySQL port.
SET NAMES utf8mb4;
CREATE TABLE `app_secrets` (
  `name` VARCHAR(255),
  `value` LONGTEXT NOT NULL,
  PRIMARY KEY (`name`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `accounts` (
  `id` VARCHAR(64),
  `external_account_id` BIGINT NOT NULL,
  `name` LONGTEXT NOT NULL,
  `cards_count` BIGINT NOT NULL DEFAULT 0,
  `auto_postpone_period_in_days` BIGINT NOT NULL DEFAULT 30,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL DEFAULT (''),
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `account_external_id_sequences` (
  `id` BIGINT,
  `value` BIGINT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `account_cancellations` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `initiated_by_id` VARCHAR(64) NOT NULL,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `cancellation_mail_jobs` (
  `id` VARCHAR(64),
  `cancellation_id` VARCHAR(64) NOT NULL,
  `recipient` LONGTEXT NOT NULL,
  `account_name` LONGTEXT NOT NULL,
  `status` BIGINT NOT NULL DEFAULT 0,
  `claimed_at` VARCHAR(40),
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `account_join_codes` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `code` VARCHAR(255) NOT NULL,
  `usage_count` BIGINT NOT NULL DEFAULT 0,
  `usage_limit` BIGINT NOT NULL DEFAULT 10,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `identities` (
  `id` VARCHAR(64),
  `email_address` VARCHAR(255) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `identity_transfers` (
  `id` VARCHAR(64),
  `identity_id` VARCHAR(64) NOT NULL,
  `token` VARCHAR(255) NOT NULL,
  `created_at` VARCHAR(40) NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `users` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `identity_id` VARCHAR(64),
  `name` LONGTEXT NOT NULL,
  `role` LONGTEXT NOT NULL DEFAULT ('member'),
  `active` BIGINT NOT NULL DEFAULT 1,
  `verified_at` LONGTEXT,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL DEFAULT (''),
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `user_attachables` (
  `user_id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `sgid` VARCHAR(255) NOT NULL,
  PRIMARY KEY (`user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `access_tokens` (
  `id` VARCHAR(64),
  `identity_id` VARCHAR(64) NOT NULL,
  `token_hash` VARCHAR(255) NOT NULL,
  `description` LONGTEXT NOT NULL,
  `permission` LONGTEXT NOT NULL CHECK(permission IN ('read','write')),
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `access_token_reveals` (
  `id` VARCHAR(64),
  `access_token_id` VARCHAR(64) NOT NULL,
  `identity_id` VARCHAR(64) NOT NULL,
  `token` LONGTEXT NOT NULL,
  `expires_at` VARCHAR(40) NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `magic_links` (
  `id` VARCHAR(64),
  `identity_id` VARCHAR(64) NOT NULL,
  `code` VARCHAR(255) NOT NULL,
  `purpose` LONGTEXT NOT NULL CHECK(purpose IN ('sign_in','sign_up')),
  `expires_at` VARCHAR(40) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `email_change_requests` (
  `token_hash` VARCHAR(255),
  `account_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `old_identity_id` VARCHAR(64) NOT NULL,
  `new_email_address` LONGTEXT NOT NULL,
  `expires_at` VARCHAR(40) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`token_hash`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `pending_authentications` (
  `token_hash` VARCHAR(255),
  `email_address` LONGTEXT NOT NULL,
  `expires_at` VARCHAR(40) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`token_hash`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `sessions` (
  `id` VARCHAR(64),
  `identity_id` VARCHAR(64) NOT NULL,
  `token_hash` VARCHAR(255) NOT NULL,
  `user_agent` LONGTEXT,
  `signup_pending` BIGINT NOT NULL DEFAULT 0,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `passkeys` (
  `id` VARCHAR(64),
  `identity_id` VARCHAR(64) NOT NULL,
  `credential_id` VARCHAR(64) NOT NULL,
  `credential_json` LONGTEXT NOT NULL,
  `name` VARCHAR(255),
  `transports` LONGTEXT NOT NULL DEFAULT ('[]'),
  `sign_count` BIGINT NOT NULL DEFAULT 0,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `passkey_challenges` (
  `challenge` VARCHAR(255),
  `purpose` LONGTEXT NOT NULL CHECK(purpose IN ('registration','authentication')),
  `identity_id` VARCHAR(64),
  `state_json` LONGTEXT NOT NULL,
  `expires_at` VARCHAR(40) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`challenge`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `authentication_attempts` (
  `id` VARCHAR(64),
  `bucket` VARCHAR(255) NOT NULL,
  `created_at` VARCHAR(40) NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `exports` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `kind` VARCHAR(32) NOT NULL CHECK(kind IN ('user','account')),
  `status` LONGTEXT NOT NULL DEFAULT ('pending') CHECK(status IN ('pending','processing','completed','failed')),
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  `completed_at` LONGTEXT,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `imports` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `identity_id` VARCHAR(64) NOT NULL,
  `status` VARCHAR(32) NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','completed','failed')),
  `failure_reason` LONGTEXT,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  `completed_at` LONGTEXT,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `boards` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `creator_id` VARCHAR(64) NOT NULL,
  `name` LONGTEXT NOT NULL,
  `all_access` BIGINT NOT NULL DEFAULT 1,
  `auto_postpone_period_in_days` BIGINT NOT NULL DEFAULT 30,
  `entropy_override` BIGINT NOT NULL DEFAULT 0,
  `public_description` LONGTEXT NOT NULL DEFAULT (''),
  `publication_key` VARCHAR(255),
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `board_publications` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `board_id` VARCHAR(64) NOT NULL,
  `key` VARCHAR(255) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `entropy_records` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `container_type` VARCHAR(255) NOT NULL CHECK(container_type IN ('Account','Board')),
  `container_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `accesses` (
  `board_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `involvement` LONGTEXT NOT NULL DEFAULT ('access_only') CHECK(involvement IN ('access_only','watching')),
  `accessed_at` VARCHAR(40),
  PRIMARY KEY (`board_id`, `user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `access_records` (
  `board_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `id` VARCHAR(64) NOT NULL,
  `account_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`board_id`, `user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `columns` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `board_id` VARCHAR(64) NOT NULL,
  `name` LONGTEXT NOT NULL,
  `color` LONGTEXT NOT NULL DEFAULT ('var(--color-card-default)'),
  `position` BIGINT NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `cards` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `board_id` VARCHAR(64) NOT NULL,
  `column_id` VARCHAR(64),
  `creator_id` VARCHAR(64) NOT NULL,
  `number` BIGINT NOT NULL,
  `title` LONGTEXT NOT NULL,
  `description` LONGTEXT NOT NULL DEFAULT (''),
  `search_text` LONGTEXT NOT NULL DEFAULT (''),
  `status` VARCHAR(32) NOT NULL DEFAULT 'published',
  `due_on` LONGTEXT,
  `closed` BIGINT NOT NULL DEFAULT 0,
  `postponed` BIGINT NOT NULL DEFAULT 0,
  `golden` BIGINT NOT NULL DEFAULT 0,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  `last_active_at` VARCHAR(40) NOT NULL,
  `bulk_touch_epoch` BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `card_bulk_touches` (
  `board_id` VARCHAR(64),
  PRIMARY KEY (`board_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `storage_blobs` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `storage_key` VARCHAR(255) NOT NULL,
  `filename` LONGTEXT NOT NULL,
  `content_type` LONGTEXT NOT NULL,
  `byte_size` BIGINT NOT NULL,
  `checksum` LONGTEXT NOT NULL,
  `metadata` LONGTEXT NOT NULL DEFAULT ('{}'),
  `signed_id` VARCHAR(64) NOT NULL,
  `attachable_sgid` VARCHAR(255) NOT NULL,
  `upload_token_hash` VARCHAR(255) NOT NULL,
  `upload_expires_at` LONGTEXT NOT NULL,
  `uploaded_at` LONGTEXT,
  `width` BIGINT,
  `height` BIGINT,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `storage_variant_records` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `blob_id` VARCHAR(64) NOT NULL,
  `variation_digest` VARCHAR(255) NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `storage_attachments` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `blob_id` VARCHAR(64) NOT NULL,
  `record_type` VARCHAR(255) NOT NULL,
  `record_id` VARCHAR(64) NOT NULL,
  `name` VARCHAR(255) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `storage_entries` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `blob_id` VARCHAR(64),
  `board_id` VARCHAR(64),
  `created_at` LONGTEXT NOT NULL,
  `delta` BIGINT NOT NULL,
  `operation` LONGTEXT NOT NULL,
  `recordable_id` VARCHAR(64),
  `recordable_type` VARCHAR(255),
  `request_id` VARCHAR(255),
  `user_id` VARCHAR(64),
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `storage_totals` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `owner_id` VARCHAR(64) NOT NULL,
  `owner_type` VARCHAR(255) NOT NULL,
  `bytes_stored` BIGINT NOT NULL DEFAULT 0,
  `last_entry_id` LONGTEXT,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `storage_download_tokens` (
  `token_hash` VARCHAR(255),
  `blob_id` VARCHAR(64) NOT NULL,
  `disposition` LONGTEXT NOT NULL DEFAULT ('inline'),
  `variant` LONGTEXT NOT NULL DEFAULT ('original'),
  `expires_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`token_hash`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `closures` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64),
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `card_goldnesses` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `card_not_nows` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64),
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `card_activity_spikes` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `steps` (
  `id` VARCHAR(64),
  `card_id` VARCHAR(64) NOT NULL,
  `content` LONGTEXT NOT NULL,
  `completed` BIGINT NOT NULL DEFAULT 0,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL DEFAULT (''),
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `comments` (
  `id` VARCHAR(64),
  `card_id` VARCHAR(64) NOT NULL,
  `creator_id` VARCHAR(64) NOT NULL,
  `body` LONGTEXT NOT NULL,
  `search_text` LONGTEXT NOT NULL DEFAULT (''),
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `rich_text_records` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `record_type` VARCHAR(255) NOT NULL,
  `record_id` VARCHAR(64) NOT NULL,
  `name` VARCHAR(255) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  `body_is_null` BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `tags` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `title` VARCHAR(255) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL DEFAULT '',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `taggings` (
  `card_id` VARCHAR(64) NOT NULL,
  `tag_id` VARCHAR(64) NOT NULL,
  `id` VARCHAR(64) NOT NULL,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL,
  PRIMARY KEY (`card_id`, `tag_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `assignments` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `assignee_id` VARCHAR(64) NOT NULL,
  `assigner_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL DEFAULT '',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `watches` (
  `id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `watching` BIGINT NOT NULL DEFAULT 1,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`card_id`, `user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `pins` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL DEFAULT '',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `reactions` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `reactable_type` VARCHAR(255) NOT NULL CHECK(reactable_type IN ('Card','Comment')),
  `reactable_id` VARCHAR(64) NOT NULL,
  `reacter_id` VARCHAR(64) NOT NULL,
  `content` LONGTEXT NOT NULL,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL DEFAULT '',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `mentions` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `mentionee_id` VARCHAR(64) NOT NULL,
  `mentioner_id` VARCHAR(64) NOT NULL,
  `source_type` VARCHAR(255) NOT NULL CHECK(source_type IN ('Card','Comment')),
  `source_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `events` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `board_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `creator_id` VARCHAR(64) NOT NULL,
  `eventable_type` VARCHAR(255) NOT NULL CHECK(eventable_type IN ('Card','Comment')),
  `eventable_id` VARCHAR(64) NOT NULL,
  `action` LONGTEXT NOT NULL,
  `particulars` LONGTEXT NOT NULL DEFAULT ('{}'),
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL DEFAULT (''),
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `notifications` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `creator_id` VARCHAR(64) NOT NULL,
  `card_id` VARCHAR(64) NOT NULL,
  `source_id` VARCHAR(64) NOT NULL,
  `source_type` VARCHAR(255) NOT NULL DEFAULT 'Event',
  `unread_count` BIGINT NOT NULL DEFAULT 1,
  `read_at` VARCHAR(40),
  `created_at` LONGTEXT NOT NULL,
  `updated_at` VARCHAR(40) NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `notification_bundles` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `starts_at` LONGTEXT NOT NULL,
  `ends_at` VARCHAR(40) NOT NULL,
  `status` BIGINT NOT NULL DEFAULT 0 CHECK(status IN (0,1,2)),
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `notification_settings` (
  `user_id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `bundle_email_frequency` LONGTEXT NOT NULL DEFAULT ('every_few_hours') CHECK(bundle_email_frequency IN ('never','every_few_hours','daily','weekly')),
  PRIMARY KEY (`user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `notification_unsubscribe_tokens` (
  `user_id` VARCHAR(64),
  `token` VARCHAR(255) NOT NULL,
  `expires_at` VARCHAR(40) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `push_subscriptions` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `endpoint` VARCHAR(255) NOT NULL,
  `p256dh_key` LONGTEXT,
  `auth_key` LONGTEXT,
  `user_agent` LONGTEXT,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `push_deliveries` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `notification_id` VARCHAR(64) NOT NULL,
  `source_id` VARCHAR(64) NOT NULL,
  `status` BIGINT NOT NULL DEFAULT 0 CHECK(status IN (0,1,2)),
  `attempts` BIGINT NOT NULL DEFAULT 0,
  `next_attempt_at` VARCHAR(40) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `push_delivery_receipts` (
  `delivery_id` VARCHAR(64) NOT NULL,
  `subscription_id` VARCHAR(64) NOT NULL,
  PRIMARY KEY (`delivery_id`, `subscription_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `user_timezones` (
  `user_id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `timezone_name` LONGTEXT,
  PRIMARY KEY (`user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `user_settings_records` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `user_id` VARCHAR(64) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `filters` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `creator_id` VARCHAR(64) NOT NULL,
  `fields` LONGTEXT NOT NULL DEFAULT ('{}'),
  `params_digest` VARCHAR(255) NOT NULL,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `filters_tags` (
  `filter_id` VARCHAR(64) NOT NULL,
  `tag_id` VARCHAR(64) NOT NULL,
  PRIMARY KEY (`filter_id`, `tag_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `boards_filters` (
  `filter_id` VARCHAR(64) NOT NULL,
  `board_id` VARCHAR(64) NOT NULL,
  PRIMARY KEY (`filter_id`, `board_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `assignees_filters` (
  `filter_id` VARCHAR(64) NOT NULL,
  `assignee_id` VARCHAR(64) NOT NULL,
  PRIMARY KEY (`filter_id`, `assignee_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `creators_filters` (
  `filter_id` VARCHAR(64) NOT NULL,
  `creator_id` VARCHAR(64) NOT NULL,
  PRIMARY KEY (`filter_id`, `creator_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `closers_filters` (
  `filter_id` VARCHAR(64) NOT NULL,
  `closer_id` VARCHAR(64) NOT NULL,
  PRIMARY KEY (`filter_id`, `closer_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `webhooks` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `board_id` VARCHAR(64) NOT NULL,
  `name` VARCHAR(255) NOT NULL,
  `url` LONGTEXT NOT NULL,
  `active` BIGINT NOT NULL DEFAULT 1,
  `signing_secret` LONGTEXT NOT NULL,
  `subscribed_actions` LONGTEXT NOT NULL DEFAULT ('[]'),
  `consecutive_failures_count` BIGINT NOT NULL DEFAULT 0,
  `first_failure_at` LONGTEXT,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `webhook_delinquency_trackers` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `webhook_id` VARCHAR(64) NOT NULL,
  `consecutive_failures_count` BIGINT NOT NULL DEFAULT 0,
  `first_failure_at` LONGTEXT,
  `created_at` LONGTEXT NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `webhook_deliveries` (
  `id` VARCHAR(64),
  `account_id` VARCHAR(64) NOT NULL,
  `webhook_id` VARCHAR(64) NOT NULL,
  `event_id` VARCHAR(64) NOT NULL,
  `state` VARCHAR(255) NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','in_progress','completed','errored')),
  `request` LONGTEXT,
  `response` LONGTEXT,
  `created_at` VARCHAR(40) NOT NULL,
  `updated_at` LONGTEXT NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE TABLE `search_migrations` (
  `key` VARCHAR(255),
  PRIMARY KEY (`key`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX `uniq_accounts_0` ON `accounts` (`external_account_id`);
CREATE INDEX `account_cancellations_due` ON `account_cancellations` (`created_at`);
CREATE UNIQUE INDEX `uniq_account_cancellations_1` ON `account_cancellations` (`account_id`);
CREATE INDEX `cancellation_mail_jobs_pending` ON `cancellation_mail_jobs` (`status`, `claimed_at`);
CREATE UNIQUE INDEX `uniq_cancellation_mail_jobs_1` ON `cancellation_mail_jobs` (`cancellation_id`);
CREATE UNIQUE INDEX `uniq_account_join_codes_0` ON `account_join_codes` (`code`);
CREATE UNIQUE INDEX `uniq_account_join_codes_1` ON `account_join_codes` (`account_id`);
CREATE UNIQUE INDEX `uniq_identities_0` ON `identities` (`email_address`);
CREATE INDEX `identity_transfers_identity_created` ON `identity_transfers` (`identity_id`, `created_at`);
CREATE UNIQUE INDEX `uniq_identity_transfers_1` ON `identity_transfers` (`token`);
CREATE UNIQUE INDEX `uniq_users_1` ON `users` (`account_id`, `identity_id`);
CREATE UNIQUE INDEX `uniq_user_attachables_0` ON `user_attachables` (`sgid`);
CREATE UNIQUE INDEX `uniq_access_tokens_0` ON `access_tokens` (`token_hash`);
CREATE INDEX `access_token_reveals_expiration` ON `access_token_reveals` (`expires_at`);
CREATE INDEX `magic_links_expiration` ON `magic_links` (`expires_at`);
CREATE UNIQUE INDEX `uniq_magic_links_1` ON `magic_links` (`code`);
CREATE INDEX `email_change_requests_expiration` ON `email_change_requests` (`expires_at`);
CREATE INDEX `pending_authentications_expiration` ON `pending_authentications` (`expires_at`);
CREATE INDEX `sessions_identity` ON `sessions` (`identity_id`);
CREATE UNIQUE INDEX `uniq_sessions_1` ON `sessions` (`token_hash`);
CREATE INDEX `passkeys_identity_name` ON `passkeys` (`identity_id`, `name`, `created_at`);
CREATE UNIQUE INDEX `uniq_passkeys_1` ON `passkeys` (`credential_id`);
CREATE INDEX `passkey_challenges_expiration` ON `passkey_challenges` (`expires_at`);
CREATE INDEX `authentication_attempts_age` ON `authentication_attempts` (`created_at`);
CREATE INDEX `authentication_attempts_bucket` ON `authentication_attempts` (`bucket`, `created_at`);
CREATE INDEX `exports_current` ON `exports` (`account_id`, `kind`, `created_at`);
CREATE INDEX `imports_pending` ON `imports` (`status`, `created_at`);
CREATE UNIQUE INDEX `boards_publication_key` ON `boards` (`publication_key`);
CREATE INDEX `boards_account` ON `boards` (`account_id`, `created_at`);
CREATE INDEX `board_publications_account` ON `board_publications` (`account_id`);
CREATE UNIQUE INDEX `uniq_board_publications_1` ON `board_publications` (`key`);
CREATE UNIQUE INDEX `uniq_board_publications_2` ON `board_publications` (`board_id`);
CREATE INDEX `entropy_records_account` ON `entropy_records` (`account_id`);
CREATE UNIQUE INDEX `uniq_entropy_records_1` ON `entropy_records` (`container_type`, `container_id`);
CREATE INDEX `accesses_user_recent` ON `accesses` (`user_id`, `accessed_at`);
CREATE INDEX `accesses_user_board` ON `accesses` (`user_id`, `board_id`);
CREATE INDEX `access_records_user_account` ON `access_records` (`user_id`, `account_id`);
CREATE INDEX `access_records_account` ON `access_records` (`account_id`);
CREATE UNIQUE INDEX `uniq_access_records_3` ON `access_records` (`id`);
CREATE INDEX `columns_board_position` ON `columns` (`board_id`, `position`);
CREATE INDEX `cards_account_latest` ON `cards` (`account_id`, `last_active_at`);
CREATE INDEX `cards_board_column_active` ON `cards` (`board_id`, `column_id`, `status`, `closed`, `postponed`, `golden`, `last_active_at`, `id`);
CREATE INDEX `cards_board_active` ON `cards` (`board_id`, `status`, `closed`, `postponed`, `last_active_at`);
CREATE UNIQUE INDEX `uniq_cards_3` ON `cards` (`account_id`, `number`);
CREATE UNIQUE INDEX `uniq_storage_blobs_0` ON `storage_blobs` (`upload_token_hash`);
CREATE UNIQUE INDEX `uniq_storage_blobs_1` ON `storage_blobs` (`attachable_sgid`);
CREATE UNIQUE INDEX `uniq_storage_blobs_2` ON `storage_blobs` (`signed_id`);
CREATE UNIQUE INDEX `uniq_storage_blobs_3` ON `storage_blobs` (`storage_key`);
CREATE UNIQUE INDEX `uniq_storage_variant_records_0` ON `storage_variant_records` (`blob_id`, `variation_digest`);
CREATE INDEX `storage_attachments_record` ON `storage_attachments` (`record_type`, `record_id`);
CREATE INDEX `storage_attachments_blob_type` ON `storage_attachments` (`account_id`, `blob_id`, `record_type`);
CREATE INDEX `storage_entries_user` ON `storage_entries` (`user_id`);
CREATE INDEX `storage_entries_request` ON `storage_entries` (`request_id`);
CREATE INDEX `storage_entries_recordable` ON `storage_entries` (`recordable_type`, `recordable_id`);
CREATE INDEX `storage_entries_blob` ON `storage_entries` (`blob_id`);
CREATE INDEX `storage_entries_board` ON `storage_entries` (`board_id`, `id`);
CREATE INDEX `storage_entries_account` ON `storage_entries` (`account_id`, `id`);
CREATE INDEX `storage_totals_account` ON `storage_totals` (`account_id`);
CREATE UNIQUE INDEX `uniq_storage_totals_1` ON `storage_totals` (`owner_type`, `owner_id`);
CREATE INDEX `closures_account_created` ON `closures` (`account_id`, `created_at`);
CREATE INDEX `closures_user_created` ON `closures` (`user_id`, `created_at`);
CREATE UNIQUE INDEX `uniq_closures_2` ON `closures` (`card_id`);
CREATE INDEX `card_goldnesses_account` ON `card_goldnesses` (`account_id`);
CREATE UNIQUE INDEX `uniq_card_goldnesses_1` ON `card_goldnesses` (`card_id`);
CREATE INDEX `card_not_nows_user` ON `card_not_nows` (`user_id`);
CREATE INDEX `card_not_nows_account` ON `card_not_nows` (`account_id`);
CREATE UNIQUE INDEX `uniq_card_not_nows_2` ON `card_not_nows` (`card_id`);
CREATE INDEX `card_activity_spikes_account_updated` ON `card_activity_spikes` (`account_id`, `updated_at`);
CREATE UNIQUE INDEX `uniq_card_activity_spikes_1` ON `card_activity_spikes` (`card_id`);
CREATE INDEX `steps_card` ON `steps` (`card_id`);
CREATE INDEX `comments_card_created` ON `comments` (`card_id`, `created_at`);
CREATE INDEX `rich_text_records_account` ON `rich_text_records` (`account_id`);
CREATE UNIQUE INDEX `uniq_rich_text_records_1` ON `rich_text_records` (`record_type`, `record_id`, `name`);
CREATE UNIQUE INDEX `uniq_tags_0` ON `tags` (`account_id`, `title`);
CREATE INDEX `assignments_assignee` ON `assignments` (`assignee_id`, `card_id`);
CREATE UNIQUE INDEX `uniq_assignments_1` ON `assignments` (`card_id`, `assignee_id`);
CREATE UNIQUE INDEX `watches_id` ON `watches` (`id`);
CREATE UNIQUE INDEX `uniq_watches_2` ON `watches` (`id`);
CREATE INDEX `pins_user_recent` ON `pins` (`user_id`, `created_at`);
CREATE UNIQUE INDEX `uniq_pins_1` ON `pins` (`user_id`, `card_id`);
CREATE INDEX `reactions_target` ON `reactions` (`reactable_type`, `reactable_id`, `created_at`);
CREATE INDEX `mentions_mentionee` ON `mentions` (`mentionee_id`);
CREATE INDEX `mentions_source` ON `mentions` (`source_type`, `source_id`);
CREATE INDEX `events_eventable` ON `events` (`eventable_type`, `eventable_id`);
CREATE INDEX `events_board_recent` ON `events` (`board_id`, `created_at`, `id`);
CREATE INDEX `events_account_recent` ON `events` (`account_id`, `created_at`, `id`);
CREATE INDEX `notifications_source` ON `notifications` (`source_type`(16), `source_id`(64));
CREATE INDEX `notifications_user_read` ON `notifications` (`user_id`, `read_at`, `updated_at`);
CREATE UNIQUE INDEX `uniq_notifications_1` ON `notifications` (`user_id`, `card_id`);
CREATE INDEX `notification_bundles_due` ON `notification_bundles` (`status`, `ends_at`);
CREATE INDEX `notification_unsubscribe_tokens_expiry` ON `notification_unsubscribe_tokens` (`expires_at`);
CREATE UNIQUE INDEX `uniq_notification_unsubscribe_tokens_1` ON `notification_unsubscribe_tokens` (`token`);
CREATE INDEX `push_subscriptions_account` ON `push_subscriptions` (`account_id`);
CREATE UNIQUE INDEX `uniq_push_subscriptions_1` ON `push_subscriptions` (`user_id`, `endpoint`);
CREATE INDEX `push_deliveries_due` ON `push_deliveries` (`status`, `next_attempt_at`);
CREATE UNIQUE INDEX `uniq_push_deliveries_1` ON `push_deliveries` (`notification_id`, `source_id`);
CREATE INDEX `user_settings_records_account` ON `user_settings_records` (`account_id`);
CREATE UNIQUE INDEX `uniq_user_settings_records_1` ON `user_settings_records` (`user_id`);
CREATE INDEX `filters_account_creator` ON `filters` (`account_id`, `creator_id`);
CREATE UNIQUE INDEX `uniq_filters_1` ON `filters` (`creator_id`, `params_digest`);
CREATE INDEX `webhooks_board_order` ON `webhooks` (`board_id`, `name`, `id`);
CREATE INDEX `webhook_delinquency_trackers_account` ON `webhook_delinquency_trackers` (`account_id`);
CREATE UNIQUE INDEX `uniq_webhook_delinquency_trackers_1` ON `webhook_delinquency_trackers` (`webhook_id`);
CREATE INDEX `webhook_deliveries_stale` ON `webhook_deliveries` (`created_at`, `id`);
CREATE INDEX `webhook_deliveries_pending` ON `webhook_deliveries` (`state`, `created_at`);
CREATE INDEX `webhook_deliveries_webhook_recent` ON `webhook_deliveries` (`webhook_id`, `created_at`, `id`);
ALTER TABLE `users` ADD COLUMN `system_account_key` VARCHAR(64) GENERATED ALWAYS AS (CASE WHEN `role`='system' THEN `account_id` END) STORED;
CREATE UNIQUE INDEX `users_system_account` ON `users` (`system_account_key`);
ALTER TABLE `storage_attachments` ADD COLUMN `card_image_record_key` VARCHAR(64) GENERATED ALWAYS AS (CASE WHEN `record_type`='Card' AND `name`='image' THEN `record_id` END) VIRTUAL, ADD COLUMN `card_image_blob_key` VARCHAR(64) GENERATED ALWAYS AS (CASE WHEN `record_type`='Card' AND `name`='image' THEN `blob_id` END) VIRTUAL, ADD COLUMN `user_avatar_record_key` VARCHAR(64) GENERATED ALWAYS AS (CASE WHEN `record_type`='User' AND `name`='avatar' THEN `record_id` END) VIRTUAL, ADD COLUMN `user_avatar_blob_key` VARCHAR(64) GENERATED ALWAYS AS (CASE WHEN `record_type`='User' AND `name`='avatar' THEN `blob_id` END) VIRTUAL, ADD COLUMN `embed_name_key` VARCHAR(191) GENERATED ALWAYS AS (CASE WHEN `name`='embed' THEN `name` END) VIRTUAL;
CREATE UNIQUE INDEX `storage_card_image_unique` ON `storage_attachments` (`card_image_record_key`);
CREATE UNIQUE INDEX `storage_card_image_blob_unique` ON `storage_attachments` (`card_image_blob_key`);
CREATE UNIQUE INDEX `storage_user_avatar_unique` ON `storage_attachments` (`user_avatar_record_key`);
CREATE UNIQUE INDEX `storage_user_avatar_blob_unique` ON `storage_attachments` (`user_avatar_blob_key`);
CREATE UNIQUE INDEX `storage_embed_unique` ON `storage_attachments` (`record_type`, `record_id`, `embed_name_key`, `blob_id`);
ALTER TABLE `account_cancellations` ADD CONSTRAINT `fk_account_cancellations_0` FOREIGN KEY (`initiated_by_id`) REFERENCES `users` (`id`);
ALTER TABLE `account_cancellations` ADD CONSTRAINT `fk_account_cancellations_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `cancellation_mail_jobs` ADD CONSTRAINT `fk_cancellation_mail_jobs_0` FOREIGN KEY (`cancellation_id`) REFERENCES `account_cancellations` (`id`) ON DELETE CASCADE;
ALTER TABLE `account_join_codes` ADD CONSTRAINT `fk_account_join_codes_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `identity_transfers` ADD CONSTRAINT `fk_identity_transfers_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`) ON DELETE CASCADE;
ALTER TABLE `users` ADD CONSTRAINT `fk_users_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`);
ALTER TABLE `users` ADD CONSTRAINT `fk_users_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `user_attachables` ADD CONSTRAINT `fk_user_attachables_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `user_attachables` ADD CONSTRAINT `fk_user_attachables_1` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `access_tokens` ADD CONSTRAINT `fk_access_tokens_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`);
ALTER TABLE `access_token_reveals` ADD CONSTRAINT `fk_access_token_reveals_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`) ON DELETE CASCADE;
ALTER TABLE `access_token_reveals` ADD CONSTRAINT `fk_access_token_reveals_1` FOREIGN KEY (`access_token_id`) REFERENCES `access_tokens` (`id`) ON DELETE CASCADE;
ALTER TABLE `magic_links` ADD CONSTRAINT `fk_magic_links_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`) ON DELETE CASCADE;
ALTER TABLE `email_change_requests` ADD CONSTRAINT `fk_email_change_requests_0` FOREIGN KEY (`old_identity_id`) REFERENCES `identities` (`id`);
ALTER TABLE `email_change_requests` ADD CONSTRAINT `fk_email_change_requests_1` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `email_change_requests` ADD CONSTRAINT `fk_email_change_requests_2` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `sessions` ADD CONSTRAINT `fk_sessions_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`) ON DELETE CASCADE;
ALTER TABLE `passkeys` ADD CONSTRAINT `fk_passkeys_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`) ON DELETE CASCADE;
ALTER TABLE `passkey_challenges` ADD CONSTRAINT `fk_passkey_challenges_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`) ON DELETE CASCADE;
ALTER TABLE `exports` ADD CONSTRAINT `fk_exports_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `exports` ADD CONSTRAINT `fk_exports_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `imports` ADD CONSTRAINT `fk_imports_0` FOREIGN KEY (`identity_id`) REFERENCES `identities` (`id`);
ALTER TABLE `imports` ADD CONSTRAINT `fk_imports_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `boards` ADD CONSTRAINT `fk_boards_0` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`);
ALTER TABLE `boards` ADD CONSTRAINT `fk_boards_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `board_publications` ADD CONSTRAINT `fk_board_publications_0` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `board_publications` ADD CONSTRAINT `fk_board_publications_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `entropy_records` ADD CONSTRAINT `fk_entropy_records_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `accesses` ADD CONSTRAINT `fk_accesses_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `accesses` ADD CONSTRAINT `fk_accesses_1` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `access_records` ADD CONSTRAINT `fk_access_records_0` FOREIGN KEY (`board_id`, `user_id`) REFERENCES `accesses` (`board_id`, `user_id`) ON DELETE CASCADE;
ALTER TABLE `access_records` ADD CONSTRAINT `fk_access_records_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `columns` ADD CONSTRAINT `fk_columns_0` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `columns` ADD CONSTRAINT `fk_columns_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `cards` ADD CONSTRAINT `fk_cards_0` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`);
ALTER TABLE `cards` ADD CONSTRAINT `fk_cards_1` FOREIGN KEY (`column_id`) REFERENCES `columns` (`id`) ON DELETE SET NULL;
ALTER TABLE `cards` ADD CONSTRAINT `fk_cards_2` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `cards` ADD CONSTRAINT `fk_cards_3` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `card_bulk_touches` ADD CONSTRAINT `fk_card_bulk_touches_0` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_blobs` ADD CONSTRAINT `fk_storage_blobs_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_variant_records` ADD CONSTRAINT `fk_storage_variant_records_0` FOREIGN KEY (`blob_id`) REFERENCES `storage_blobs` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_variant_records` ADD CONSTRAINT `fk_storage_variant_records_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_attachments` ADD CONSTRAINT `fk_storage_attachments_0` FOREIGN KEY (`blob_id`) REFERENCES `storage_blobs` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_attachments` ADD CONSTRAINT `fk_storage_attachments_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_entries` ADD CONSTRAINT `fk_storage_entries_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_totals` ADD CONSTRAINT `fk_storage_totals_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `storage_download_tokens` ADD CONSTRAINT `fk_storage_download_tokens_0` FOREIGN KEY (`blob_id`) REFERENCES `storage_blobs` (`id`) ON DELETE CASCADE;
ALTER TABLE `closures` ADD CONSTRAINT `fk_closures_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE SET NULL;
ALTER TABLE `closures` ADD CONSTRAINT `fk_closures_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `closures` ADD CONSTRAINT `fk_closures_2` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `card_goldnesses` ADD CONSTRAINT `fk_card_goldnesses_0` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `card_goldnesses` ADD CONSTRAINT `fk_card_goldnesses_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `card_not_nows` ADD CONSTRAINT `fk_card_not_nows_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE SET NULL;
ALTER TABLE `card_not_nows` ADD CONSTRAINT `fk_card_not_nows_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `card_not_nows` ADD CONSTRAINT `fk_card_not_nows_2` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `card_activity_spikes` ADD CONSTRAINT `fk_card_activity_spikes_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `steps` ADD CONSTRAINT `fk_steps_0` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `comments` ADD CONSTRAINT `fk_comments_0` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`);
ALTER TABLE `comments` ADD CONSTRAINT `fk_comments_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `rich_text_records` ADD CONSTRAINT `fk_rich_text_records_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `tags` ADD CONSTRAINT `fk_tags_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `taggings` ADD CONSTRAINT `fk_taggings_0` FOREIGN KEY (`tag_id`) REFERENCES `tags` (`id`) ON DELETE CASCADE;
ALTER TABLE `taggings` ADD CONSTRAINT `fk_taggings_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `assignments` ADD CONSTRAINT `fk_assignments_0` FOREIGN KEY (`assigner_id`) REFERENCES `users` (`id`);
ALTER TABLE `assignments` ADD CONSTRAINT `fk_assignments_1` FOREIGN KEY (`assignee_id`) REFERENCES `users` (`id`);
ALTER TABLE `assignments` ADD CONSTRAINT `fk_assignments_2` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `assignments` ADD CONSTRAINT `fk_assignments_3` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `watches` ADD CONSTRAINT `fk_watches_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`);
ALTER TABLE `watches` ADD CONSTRAINT `fk_watches_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `pins` ADD CONSTRAINT `fk_pins_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`);
ALTER TABLE `pins` ADD CONSTRAINT `fk_pins_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `pins` ADD CONSTRAINT `fk_pins_2` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `reactions` ADD CONSTRAINT `fk_reactions_0` FOREIGN KEY (`reacter_id`) REFERENCES `users` (`id`);
ALTER TABLE `reactions` ADD CONSTRAINT `fk_reactions_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `mentions` ADD CONSTRAINT `fk_mentions_0` FOREIGN KEY (`mentioner_id`) REFERENCES `users` (`id`);
ALTER TABLE `mentions` ADD CONSTRAINT `fk_mentions_1` FOREIGN KEY (`mentionee_id`) REFERENCES `users` (`id`);
ALTER TABLE `mentions` ADD CONSTRAINT `fk_mentions_2` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `events` ADD CONSTRAINT `fk_events_0` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`);
ALTER TABLE `events` ADD CONSTRAINT `fk_events_1` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `events` ADD CONSTRAINT `fk_events_2` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `events` ADD CONSTRAINT `fk_events_3` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`);
ALTER TABLE `notifications` ADD CONSTRAINT `fk_notifications_0` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE;
ALTER TABLE `notifications` ADD CONSTRAINT `fk_notifications_1` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`);
ALTER TABLE `notifications` ADD CONSTRAINT `fk_notifications_2` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `notifications` ADD CONSTRAINT `fk_notifications_3` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `notification_bundles` ADD CONSTRAINT `fk_notification_bundles_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `notification_bundles` ADD CONSTRAINT `fk_notification_bundles_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `notification_settings` ADD CONSTRAINT `fk_notification_settings_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `notification_settings` ADD CONSTRAINT `fk_notification_settings_1` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `notification_unsubscribe_tokens` ADD CONSTRAINT `fk_notification_unsubscribe_tokens_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `push_subscriptions` ADD CONSTRAINT `fk_push_subscriptions_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `push_subscriptions` ADD CONSTRAINT `fk_push_subscriptions_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `push_deliveries` ADD CONSTRAINT `fk_push_deliveries_0` FOREIGN KEY (`notification_id`) REFERENCES `notifications` (`id`) ON DELETE CASCADE;
ALTER TABLE `push_deliveries` ADD CONSTRAINT `fk_push_deliveries_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `push_delivery_receipts` ADD CONSTRAINT `fk_push_delivery_receipts_0` FOREIGN KEY (`subscription_id`) REFERENCES `push_subscriptions` (`id`) ON DELETE CASCADE;
ALTER TABLE `push_delivery_receipts` ADD CONSTRAINT `fk_push_delivery_receipts_1` FOREIGN KEY (`delivery_id`) REFERENCES `push_deliveries` (`id`) ON DELETE CASCADE;
ALTER TABLE `user_timezones` ADD CONSTRAINT `fk_user_timezones_0` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `user_timezones` ADD CONSTRAINT `fk_user_timezones_1` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `user_settings_records` ADD CONSTRAINT `fk_user_settings_records_0` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `user_settings_records` ADD CONSTRAINT `fk_user_settings_records_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `filters` ADD CONSTRAINT `fk_filters_0` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `filters` ADD CONSTRAINT `fk_filters_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `filters_tags` ADD CONSTRAINT `fk_filters_tags_0` FOREIGN KEY (`tag_id`) REFERENCES `tags` (`id`) ON DELETE CASCADE;
ALTER TABLE `filters_tags` ADD CONSTRAINT `fk_filters_tags_1` FOREIGN KEY (`filter_id`) REFERENCES `filters` (`id`) ON DELETE CASCADE;
ALTER TABLE `boards_filters` ADD CONSTRAINT `fk_boards_filters_0` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `boards_filters` ADD CONSTRAINT `fk_boards_filters_1` FOREIGN KEY (`filter_id`) REFERENCES `filters` (`id`) ON DELETE CASCADE;
ALTER TABLE `assignees_filters` ADD CONSTRAINT `fk_assignees_filters_0` FOREIGN KEY (`assignee_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `assignees_filters` ADD CONSTRAINT `fk_assignees_filters_1` FOREIGN KEY (`filter_id`) REFERENCES `filters` (`id`) ON DELETE CASCADE;
ALTER TABLE `creators_filters` ADD CONSTRAINT `fk_creators_filters_0` FOREIGN KEY (`creator_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `creators_filters` ADD CONSTRAINT `fk_creators_filters_1` FOREIGN KEY (`filter_id`) REFERENCES `filters` (`id`) ON DELETE CASCADE;
ALTER TABLE `closers_filters` ADD CONSTRAINT `fk_closers_filters_0` FOREIGN KEY (`closer_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;
ALTER TABLE `closers_filters` ADD CONSTRAINT `fk_closers_filters_1` FOREIGN KEY (`filter_id`) REFERENCES `filters` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhooks` ADD CONSTRAINT `fk_webhooks_0` FOREIGN KEY (`board_id`) REFERENCES `boards` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhooks` ADD CONSTRAINT `fk_webhooks_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhook_delinquency_trackers` ADD CONSTRAINT `fk_webhook_delinquency_trackers_0` FOREIGN KEY (`webhook_id`) REFERENCES `webhooks` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhook_delinquency_trackers` ADD CONSTRAINT `fk_webhook_delinquency_trackers_1` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhook_deliveries` ADD CONSTRAINT `fk_webhook_deliveries_0` FOREIGN KEY (`event_id`) REFERENCES `events` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhook_deliveries` ADD CONSTRAINT `fk_webhook_deliveries_1` FOREIGN KEY (`webhook_id`) REFERENCES `webhooks` (`id`) ON DELETE CASCADE;
ALTER TABLE `webhook_deliveries` ADD CONSTRAINT `fk_webhook_deliveries_2` FOREIGN KEY (`account_id`) REFERENCES `accounts` (`id`) ON DELETE CASCADE;
