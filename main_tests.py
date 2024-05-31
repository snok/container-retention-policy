import asyncio
import os
import tempfile
from asyncio import Semaphore
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from unittest.mock import ANY, AsyncMock, Mock

import pytest as pytest
from httpx import AsyncClient
from pydantic import ValidationError

import main
from main import (
    MAX_SLEEP,
    AccountType,
    Inputs,
    MetadataModel,
    PackageResponse,
    PackageVersionResponse,
    delete_org_package_versions,
    delete_package_versions,
    filter_image_names,
    get_and_delete_old_versions,
    list_org_package_versions,
    list_package_versions,
)
from main import main as main_
from main import post_deletion_output, wait_for_rate_limit


@pytest.fixture
def ok_response():
    mock_ok_response = Mock()
    mock_ok_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_ok_response.json.return_value = []
    mock_ok_response.is_error = False
    yield mock_ok_response


@pytest.fixture
def bad_response():
    mock_bad_response = Mock()
    mock_bad_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_bad_response.is_error = True
    yield mock_bad_response


@pytest.fixture
def http_client(ok_response):
    mock_http_client = AsyncMock()
    mock_http_client.get.return_value = ok_response
    mock_http_client.delete.return_value = ok_response
    yield mock_http_client


@pytest.fixture(autouse=True)
def github_output():
    """
    Create a GITHUB_OUTPUT env value to mock the Github actions equivalent.
    """
    with tempfile.NamedTemporaryFile() as temp:
        os.environ['GITHUB_OUTPUT'] = temp.name
        yield


async def test_list_org_package_version(http_client):
    await list_org_package_versions(org_name='test', image_name='test', http_client=http_client)


async def test_wait_for_rate_limit(ok_response, capsys):
    # No rate limit hit, no secondary limit
    start = datetime.now()
    await wait_for_rate_limit(response=ok_response, eligible_for_secondary_limit=False)
    assert capsys.readouterr().out == ''  # no output
    assert (datetime.now() - start).seconds == 0

    # No rate limit hit, with secondary limit - this should sleep for one second
    start = datetime.now()
    await wait_for_rate_limit(response=ok_response, eligible_for_secondary_limit=True)
    assert capsys.readouterr().out == ''  # no output
    assert (datetime.now() - start).seconds == 1  # ~1 second runtime

    # Run with timeout exceeding max limit - this should exit the program
    ok_response.headers = {'x-ratelimit-remaining': '0'}
    ok_response.headers |= {'x-ratelimit-reset': (datetime.now() + timedelta(seconds=MAX_SLEEP + 1)).timestamp()}
    with pytest.raises(SystemExit):
        await wait_for_rate_limit(response=ok_response)
    assert " Terminating workflow, since that's above the maximum allowed sleep time" in capsys.readouterr().out

    # Run with timeout below max limit - this should just sleep for a bit
    ok_response.headers |= {'x-ratelimit-reset': (datetime.now() + timedelta(seconds=2)).timestamp()}
    await wait_for_rate_limit(response=ok_response)
    assert 'Rate limit exceeded. Sleeping for' in capsys.readouterr().out


async def test_list_package_version(http_client):
    await list_package_versions(image_name='test', http_client=http_client)


async def test_delete_org_package_version(http_client):
    await delete_org_package_versions(
        org_name='test',
        image_name='test',
        http_client=http_client,
        version_id=123,
        semaphore=Semaphore(1),
    )


async def test_delete_package_version(http_client):
    await delete_package_versions(image_name='test', http_client=http_client, version_id=123, semaphore=Semaphore(1))


async def test_delete_package_version_semaphore(http_client):
    """
    A bit of a useless test, but proves Semaphores work the way we think.
    """
    # Test that we're still waiting after 1 second, when the semaphore is empty
    sem = Semaphore(0)
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(
            delete_package_versions(image_name='test', http_client=http_client, version_id=123, semaphore=sem),
            2,
        )

    # Assert that this would not be the case otherwise
    sem = Semaphore(1)
    await asyncio.wait_for(
        delete_package_versions(image_name='test', http_client=http_client, version_id=123, semaphore=sem),
        2,
    )


def test_post_deletion_output(capsys, ok_response, bad_response):
    # Happy path
    post_deletion_output(response=ok_response, image_name='test', version_id=123)
    captured = capsys.readouterr()
    assert captured.out == 'Deleted old image: test:123\n'

    # Bad response
    post_deletion_output(response=bad_response, image_name='test', version_id=123)
    captured = capsys.readouterr()
    assert captured.out != 'Deleted old image: test:123\n'


input_defaults = {
    'image_names': 'a,b',
    'cut_off': 'an hour ago utc',
    'timestamp_to_use': 'created_at',
    'untagged_only': 'false',
    'skip_tags': '',
    'filter_tags': '',
    'filter_include_untagged': 'true',
    'token': 'test',
    'token_type': 'pat',
    'account_type': 'personal',
    'dry_run': 'false',
}


def _create_inputs_model(**kwargs):
    """
    Little helper method, to help us instantiate working Inputs models.
    """

    return Inputs(**(input_defaults | kwargs))


def test_org_name_empty():
    with pytest.raises(ValidationError):
        Inputs(**(input_defaults | {'account_type': 'org', 'org_name': ''}))


async def test_inputs_model_personal(mocker):
    # Mock the personal list function
    mocked_list_package_versions: AsyncMock = mocker.patch.object(main, 'list_package_versions', AsyncMock())
    mocked_delete_package_versions: AsyncMock = mocker.patch.object(main, 'delete_package_versions', AsyncMock())

    # Create a personal inputs model
    personal = _create_inputs_model(account_type='personal')
    assert personal.account_type != AccountType.ORG

    # Call the GithubAPI utility function
    await main.GithubAPI.list_package_versions(
        account_type=personal.account_type,
        org_name=personal.org_name,
        image_name=personal.image_names[0],
        http_client=AsyncMock(),
    )
    await main.GithubAPI.delete_package(
        account_type=personal.account_type,
        org_name=personal.org_name,
        image_name=personal.image_names[0],
        http_client=AsyncMock(),
        version_id=1,
        semaphore=Semaphore(1),
    )

    # Make sure the right function was called
    mocked_list_package_versions.assert_awaited_once()
    mocked_delete_package_versions.assert_awaited_once()


async def test_inputs_model_org(mocker):
    # Mock the org list function
    mocked_list_package_versions: AsyncMock = mocker.patch.object(main, 'list_org_package_versions', AsyncMock())
    mocked_delete_package_versions: AsyncMock = mocker.patch.object(main, 'delete_org_package_versions', AsyncMock())

    # Create a personal inputs model
    org = _create_inputs_model(account_type='org', org_name='test')
    assert org.account_type == AccountType.ORG

    # Call the GithubAPI utility function
    await main.GithubAPI.list_package_versions(
        account_type=org.account_type, org_name=org.org_name, image_name=org.image_names[0], http_client=AsyncMock()
    )
    await main.GithubAPI.delete_package(
        account_type=org.account_type,
        org_name=org.org_name,
        image_name=org.image_names[0],
        http_client=AsyncMock(),
        version_id=1,
        semaphore=Semaphore(1),
    )

    # Make sure the right function was called
    mocked_list_package_versions.assert_awaited_once()
    mocked_delete_package_versions.assert_awaited_once()


class TestGetAndDeleteOldVersions:
    valid_data = [
        PackageVersionResponse(
            **{
                'id': 1234567,
                'name': 'sha256:3c6891187412bd31fa04c63b4f06c47417eb599b1b659462632285531aa99c19',
                'created_at': '2021-05-26T14:03:03Z',
                'updated_at': '2021-05-26T14:03:03Z',
                'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                'html_url': 'https://github.com/orgs/org-name/packages/container/image-name/1234567',
                'package_html_url': 'https://github.com/orgs/org-name/packages/container/package/image-name',
                'url': 'https://api.github.com/orgs/org-name/packages/container/image-name/versions/1234567',
            }
        )
    ]

    @staticmethod
    def generate_fresh_valid_data_with_id(id):
        r = deepcopy(TestGetAndDeleteOldVersions.valid_data[0])
        r.id = id
        r.created_at = datetime.now(timezone(timedelta()))
        return r

    async def test_delete_package(self, mocker, capsys, http_client):
        # Mock the list function
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=self.valid_data)

        # Call the function
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)

        # Check the output
        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'


    async def test_not_beyond_cutoff(self, mocker, capsys, http_client):
        response_data = [
            PackageVersionResponse(
                created_at=datetime.now(timezone(timedelta(hours=1))),
                updated_at=datetime.now(timezone(timedelta(hours=1))),
                id=1234567,
                name='',
                metadata={'container': {'tags': []}, 'package_type': 'container'},
            )
        ]
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=response_data)
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_missing_timestamp(self, mocker, capsys, http_client):
        data = [
            PackageVersionResponse(
                created_at=None,
                updated_at=None,
                id=1234567,
                name='',
                metadata={'container': {'tags': []}, 'package_type': 'container'},
            )
        ]
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert (
            captured.out
            == 'Skipping image version 1234567. Unable to parse timestamps.\nNo more versions to delete for a\n'
        )

    async def test_empty_list(self, mocker, capsys, http_client):
        data = []
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_skip_tags(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(**{'container': {'tags': ['abc', 'bcd']}, 'package_type': 'container'})
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(skip_tags='abc')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_skip_tags_wildcard(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(**{'container': {'tags': ['v1.0.0', 'abc']}, 'package_type': 'container'})
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(skip_tags='v*')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_untagged_only(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(**{'container': {'tags': ['abc', 'bcd']}, 'package_type': 'container'})
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(untagged_only='true')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_filter_tags(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(
            **{'container': {'tags': ['sha-deadbeef', 'edge']}, 'package_type': 'container'}
        )
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(filter_tags='sha-*')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'

    async def test_dry_run(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(
            **{'container': {'tags': ['sha-deadbeef', 'edge']}, 'package_type': 'container'}
        )
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        mock_delete_package = mocker.patch.object(main.GithubAPI, 'delete_package')
        inputs = _create_inputs_model(dry_run='true')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Would delete image a:1234567.\nNo more versions to delete for a\n'
        mock_delete_package.assert_not_called()


def test_inputs_bad_token_type():
    with pytest.raises(ValidationError, match='Input should be \'github-token\' or \'pat\''):
        _create_inputs_model(token_type='undefined-token-type', image_names='a,b')


def test_inputs_token_type_as_github_token_with_bad_image_names():
    _create_inputs_model(image_names='a', token_type='github-token')
    with pytest.raises(ValidationError, match='Wildcards are not allowed if token_type is github-token'):
        _create_inputs_model(image_names='a*', token_type='github-token')
    with pytest.raises(ValidationError, match='A single image name is required if token_type is github-token'):
        _create_inputs_model(image_names='a,b,c', token_type='github-token')


async def test_outputs_are_set(mocker):
    for i in [
        'deleted=',
        'failed=',
    ]:
        assert i in out_vars
