import clsx from 'clsx';
import { useEffect, useState } from 'react';
import Session, { signOut } from 'supertokens-web-js/recipe/session';
import { auth, useBridgeMutation, useBridgeQuery, useFeatureFlag } from '@sd/client';
import { Button, Input, toast } from '@sd/ui';
import { Authentication } from '~/components';
import { useLocale } from '~/hooks';
import { AUTH_SERVER_URL, getTokens } from '~/util';

import { Heading } from '../../Layout';
import Profile from './Profile';

type User = {
	email: string;
	id: string;
	timejoined: number;
	roles: string[];
};

export const Component = () => {
	const { t } = useLocale();
	const [userInfo, setUserInfo] = useState<User | null>(null);
	const [reload, setReload] = useState(false);

	useEffect(() => {
		async function _() {
			const user_data = await fetch(`${AUTH_SERVER_URL}/api/user`, {
				method: 'GET'
			});
			const data = await user_data.json();
			console.log('Data from user (auth API)', data);
			return data;
		}
		_().then((data) => {
			// Check if data is the same as the user type
			if (data.id) {
				setUserInfo(data);
			} else {
				setUserInfo(null);
			}
		});
		setReload(false);
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [reload]);
	const cloudBootstrap = useBridgeMutation('cloud.bootstrap');
	const tokens = getTokens();

	return (
		<>
			<Heading
				title={t('spacedrive_account')}
				description={t('spacedrive_cloud_description')}
			/>
			<div className={clsx(userInfo != null ? '' : 'flex items-center justify-center')}>
				<div
					className={clsx(
						userInfo != null ? '' : 'w-full max-w-md space-y-8 p-8 text-center lg:p-12'
					)}
				>
					{userInfo === null ? (
						<>
							<Authentication reload={setReload} cloudBootstrap={cloudBootstrap} />
						</>
					) : (
						<>
							<Profile user={userInfo} setReload={setReload} />
						</>
					)}
				</div>
			</div>
			{/* {useFeatureFlag('hostedLocations') && <HostedLocationsPlayground />} */}
		</>
	);
};

// Not supporting this feature for now
// function HostedLocationsPlayground() {
// 	const locations = useBridgeQuery(['cloud.locations.list'], { retry: false });

// 	const [locationName, setLocationName] = useState('');
// 	const [path, setPath] = useState('');
// 	const createLocation = useBridgeMutation('cloud.locations.create', {
// 		onSuccess(data) {
// 			// console.log('DATA', data); // TODO: Optimistic UI

// 			locations.refetch();
// 			setLocationName('');
// 		}
// 	});
// 	const removeLocation = useBridgeMutation('cloud.locations.remove', {
// 		onSuccess() {
// 			// TODO: Optimistic UI

// 			locations.refetch();
// 		}
// 	});

// 	useEffect(() => {
// 		if (path === '' && locations.data?.[0]) {
// 			setPath(`location/${locations.data[0].id}/hello.txt`);
// 		}
// 	}, [path, locations.data]);

// 	const isLoading = createLocation.isLoading || removeLocation.isLoading;

// 	return (
// 		<>
// 			<Heading
// 				rightArea={
// 					<div className="flex-row space-x-2">
// 						{/* TODO: We need UI for this. I wish I could use `prompt` for now but Tauri doesn't have it :( */}
// 						<div className="flex flex-row space-x-4">
// 							<Input
// 								className="grow"
// 								value={locationName}
// 								onInput={(e) => setLocationName(e.currentTarget.value)}
// 								placeholder="My sick location"
// 								disabled={isLoading}
// 							/>

// 							<Button
// 								variant="accent"
// 								size="sm"
// 								onClick={() => {
// 									if (locationName === '') return;
// 									createLocation.mutate(locationName);
// 								}}
// 								disabled={isLoading}
// 							>
// 								Create Location
// 							</Button>
// 						</div>
// 					</div>
// 				}
// 				title="Hosted Locations"
// 				description="Augment your local storage with our cloud!"
// 			/>

// 			{/* TODO: Cleanup this mess + styles */}
// 			{locations.status === 'loading' ? <div>Loading!</div> : null}
// 			{locations.status !== 'loading' && locations.data?.length === 0 ? (
// 				<div>Looks like you don't have any!</div>
// 			) : (
// 				<div>
// 					{locations.data?.map((location) => (
// 						<div key={location.id} className="flex flex-row space-x-5">
// 							<h1>{location.name}</h1>
// 							<Button
// 								variant="accent"
// 								size="sm"
// 								onClick={() => removeLocation.mutate(location.id)}
// 								disabled={isLoading}
// 							>
// 								Delete
// 							</Button>
// 						</div>
// 					))}
// 				</div>
// 			)}

// 			<div>
// 				<p>Path to save when clicking 'Do the thing':</p>
// 				<Input
// 					className="grow"
// 					value={path}
// 					onInput={(e) => setPath(e.currentTarget.value)}
// 					disabled={isLoading}
// 				/>
// 			</div>
// 		</>
// 	);
// }
