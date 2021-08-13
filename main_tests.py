from datetime import datetime, timedelta, timezone
from functools import partial
from pathlib import Path
from unittest.mock import AsyncMock, Mock

import pytest as pytest
from dateparser import parse
from httpx import AsyncClient

import main
from main import (
    AccountType,
    ImageName,
    Inputs,
    TimestampType,
    delete_org_package_versions,
    delete_package_versions,
    get_and_delete_old_versions,
    list_org_package_versions,
    list_package_versions,
)
from main import main as main_
from main import parse_image_names, validate_inputs

mock_response = Mock()
mock_response.json.return_value = []
mock_response.is_error = False
mock_http_client = AsyncMock()
mock_http_client.get.return_value = mock_response
mock_http_client.delete.return_value = mock_response


@pytest.mark.asyncio
async def test_list_org_package_version():
    await list_org_package_versions(org_name='test', image_name=ImageName('test', 'test'), http_client=mock_http_client)


@pytest.mark.asyncio
async def test_list_package_version():
    await list_package_versions(image_name=ImageName('test', 'test'), http_client=mock_http_client)


@pytest.mark.asyncio
async def test_delete_org_package_version():
    await delete_org_package_versions(org_name='test', image_name=ImageName('test', 'test'), http_client=mock_http_client, version_id=123)


@pytest.mark.asyncio
async def test_delete_package_version():
    await delete_package_versions(image_name=ImageName('test', 'test'), http_client=mock_http_client, version_id=123)


def test_inputs_dataclass():
    personal = Inputs(
        parsed_cutoff=parse('an hour ago utc'), timestamp_type=TimestampType('created_at'), account_type=AccountType('personal')
    )
    assert personal.is_org is False
    assert personal.list_package_versions == list_package_versions
    assert personal.delete_package == delete_package_versions

    org = Inputs(
        parsed_cutoff=parse('an hour ago utc'),
        timestamp_type=TimestampType('created_at'),
        account_type=AccountType('org'),
        org_name='abcorp',
    )
    assert org.is_org is True
    assert isinstance(org.list_package_versions, partial)
    assert isinstance(org.delete_package, partial)


class TestGetAndDeleteOldVersions:
    @staticmethod
    async def mock_list_package_versions(data, *args):
        return data

    @pytest.mark.asyncio
    async def test_get_and_delete_old_versions_delete_package_scenario(self, capsys):
        data = [
            {
                'created_at': '2021-05-26T14:03:03Z',
                'html_url': 'https://github.com/orgs/org-name/packages/container/image-name/1234567',
                'id': 1234567,
                'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                'name': 'sha256:3c6891187412bd31fa04c63b4f06c47417eb599b1b659462632285531aa99c19',
                'package_html_url': 'https://github.com/orgs/org-name/packages/container/package/image-name',
                'updated_at': '2021-05-26T14:03:03Z',
                'url': 'https://api.github.com/orgs/org-name/packages/container/image-name/versions/1234567',
            }
        ]
        Inputs.list_package_versions = partial(self.mock_list_package_versions, data)
        inputs = Inputs(
            parsed_cutoff=parse('an hour ago utc'), timestamp_type=TimestampType('created_at'), account_type=AccountType('personal')
        )

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'

    @pytest.mark.asyncio
    async def test_get_and_delete_old_versions_not_old_enough_scenario(self, capsys):
        Inputs.list_package_versions = partial(
            self.mock_list_package_versions,
            [
                {
                    'created_at': str(datetime.now(timezone(timedelta(hours=1)))),
                    'id': 1234567,
                }
            ],
        )
        inputs = Inputs(
            parsed_cutoff=parse('2 days ago utc'), timestamp_type=TimestampType('created_at'), account_type=AccountType('personal')
        )

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_get_and_delete_old_versions_skip_package_scenario(self, capsys):
        Inputs.list_package_versions = partial(self.mock_list_package_versions, [{'created_at': '', 'id': 1234567}])
        inputs = Inputs(
            parsed_cutoff=parse('an hour ago utc'), timestamp_type=TimestampType('created_at'), account_type=AccountType('personal')
        )

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Skipping image version 1234567. Unable to parse timestamps.\nNo more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_get_and_delete_old_versions_no_packages_scenario(self, capsys):
        Inputs.list_package_versions = partial(self.mock_list_package_versions, [])
        inputs = Inputs(
            parsed_cutoff=parse('an hour ago utc'), timestamp_type=TimestampType('created_at'), account_type=AccountType('personal')
        )

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'


def test_inputs_bad_account_type():
    defaults = {'account_type': 'org', 'org_name': 'test', 'timestamp_type': 'updated_at', 'cut_off': '2 hours ago UTC'}

    # Account type
    validate_inputs(**defaults | {'account_type': 'personal'})
    validate_inputs(**defaults | {'account_type': 'org'})
    with pytest.raises(ValueError, match="'' is not a valid AccountType"):
        validate_inputs(**defaults | {'account_type': ''})

    # Org name
    validate_inputs(**defaults | {'org_name': '', 'account_type': 'personal'})
    with pytest.raises(ValueError, match='org-name is required when account-type is org'):
        validate_inputs(**defaults | {'org_name': ''})

    # Timestamp type
    validate_inputs(**defaults | {'timestamp_type': 'updated_at'})
    validate_inputs(**defaults | {'timestamp_type': 'created_at'})
    with pytest.raises(ValueError, match="'wat' is not a valid TimestampType"):
        validate_inputs(**defaults | {'timestamp_type': 'wat'})

    # Cut-off
    validate_inputs(**defaults | {'cut_off': '21 July 2013 10:15 pm +0500'})
    validate_inputs(**defaults | {'cut_off': '12/12/12 PM EST'})
    with pytest.raises(ValueError, match='Timezone is required for the cut-off'):
        validate_inputs(**defaults | {'cut_off': '12/12/12'})
    with pytest.raises(ValueError, match="Unable to parse 'lolol'"):
        validate_inputs(**defaults | {'cut_off': 'lolol'})


def test_parse_image_names():
    assert parse_image_names('a') == [ImageName('a', 'a')]
    assert parse_image_names('a,b') == [ImageName('a', 'a'), ImageName('b', 'b')]
    assert parse_image_names('  a  ,  b ') == [ImageName('a', 'a'), ImageName('b', 'b')]
    assert parse_image_names('a/a') == [ImageName('a/a', 'a%2Fa')]


@pytest.mark.asyncio
async def test_main(mocker):
    mocker.patch.object(AsyncClient, 'get', return_value=mock_response)
    mocker.patch.object(AsyncClient, 'delete', return_value=mock_response)
    mocker.patch.object(main, 'get_and_delete_old_versions', AsyncMock())
    await main_(
        **{
            'account_type': 'org',
            'org_name': 'test',
            'image_names': 'a,b,c',
            'timestamp_type': 'updated_at',
            'cut_off': '2 hours ago UTC',
            'token': 'abc',
        }
    )
