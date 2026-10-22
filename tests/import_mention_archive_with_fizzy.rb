# Import a disposable Rizzy ZIP into the pinned Fizzy database, then verify
# that its portable attachment GIDs resolve to the imported records.
require "digest"
require "json"
require "nokogiri"

archive = File.expand_path(ENV.fetch("RIZZY_ACCOUNT_ARCHIVE"))
expected_sha256 = ENV.fetch("EXPECTED_FILE_SHA256")
expected_blob_hashes = JSON.parse(ENV.fetch("EXPECTED_BLOB_SHA256"))
expected_attachment_blobs = JSON.parse(ENV.fetch("EXPECTED_ATTACHMENTS"))
expected_variant_count = Integer(ENV.fetch("EXPECTED_VARIANT_COUNT"))
image_filename = ENV["EXPECTED_IMAGE_FILENAME"]
database = File.expand_path(ActiveRecord::Base.connection_db_config.database)
live_database = Rails.root.join("storage/production.sqlite3").to_s
raise "Refusing the live Fizzy database" if database == live_database
account = Account.create!(name: "Rizzy mention import probe")

File.open(archive, "rb") do |file|
  zip = ZipFile::Reader.new(file)
  Account::DataTransfer::Manifest.new(account).each_record_set do |record_set|
    record_set.import(from: zip)
  end
end

card = account.cards.find_by!(title: "Record fields")
rich_text = ActionText::RichText.find_by!(account: account, record_type: "Card", record_id: card.id, name: "description")
fragment = Nokogiri::HTML.fragment(rich_text.body.to_s)
attachments = fragment.css("action-text-attachment[sgid]")
raise "Missing imported attachments" unless attachments.size == (image_filename ? 3 : 2)
raise "Portable GID left unresolved" if fragment.to_html.include?("gid://fizzy/")
resolved = attachments.map do |node|
  SignedGlobalID.parse(node["sgid"], for: ActionText::Attachable::LOCATOR_NAME)&.find
end
user = resolved.find { |record| record.is_a?(User) }
blob = resolved.find { |record| record.is_a?(ActiveStorage::Blob) && record.filename.to_s == "notes.txt" }
raise "User mention points outside imported account" unless user&.account_id == account.id
raise "File attachment points outside imported account" unless blob&.account_id == account.id
raise "Imported file bytes changed" unless Digest::SHA256.hexdigest(blob.download) == expected_sha256
if image_filename
  image = resolved.find { |record| record.is_a?(ActiveStorage::Blob) && record.filename.to_s == image_filename }
  raise "Image attachment points outside imported account" unless image&.account_id == account.id
end
expected_blob_hashes.each do |id, expected_hash|
  imported_blob = ActiveStorage::Blob.find_by!(id: id, account: account)
  raise "Imported blob bytes changed: #{id}" unless Digest::SHA256.hexdigest(imported_blob.download) == expected_hash
end
variant_count = ActiveStorage::VariantRecord.where(account_id: account.id).count
raise "Imported variant count changed" unless variant_count == expected_variant_count
attachment_blobs = ActiveStorage::Attachment.where(account_id: account.id).pluck(:id, :blob_id).to_h
raise "Imported attachment associations changed" unless attachment_blobs == expected_attachment_blobs
mention = Mention.find_by!(account: account, source: card, mentionee: user)
raise "Imported mentioner points outside account" unless mention.mentioner.account_id == account.id

puts JSON.generate(cards: account.cards.count, mentions: Mention.where(account: account).count,
                   attachments: attachments.size, file_sha256: expected_sha256,
                   blobs: expected_blob_hashes.size, variants: variant_count,
                   storage_attachments: attachment_blobs.size, image: !!image_filename)
