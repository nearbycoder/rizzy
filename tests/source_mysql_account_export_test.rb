require "test_helper"

class SourceMysqlAccountExportTest < ActiveSupport::TestCase
  test "write a pinned Fizzy fixture account ZIP for Rizzy import" do
    blob = ActiveStorage::Blob.create_and_upload!(
      io: file_fixture("moon.jpg").open,
      filename: "moon.jpg",
      content_type: "image/jpeg"
    )
    cards(:logo).image.attach(blob)
    assert cards(:logo).image.attached?
    export = Account::Export.create!(account: Current.account, user: users(:jason))
    export.build
    assert export.completed?
    export.file.open do |file|
      entries = ZipKit::FileReader.read_zip_structure(io: file).map(&:filename)
      assert_includes entries, "storage/#{blob.key}"
      assert_includes entries, "data/active_storage_attachments/#{cards(:logo).image.attachment.id}.json"
      file.rewind
      File.binwrite(ENV.fetch("RIZZY_SOURCE_EXPORT_PATH"), file.read)
    end
  end
end
